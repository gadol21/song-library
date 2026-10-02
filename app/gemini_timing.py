import json
import os
import re
import shutil
import tempfile
import time
from datetime import date
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from google import genai
from google.genai import errors as genai_errors
from google.genai import types

from app import youtube
from app.youtube import AudioDownloadError

GEMINI_MODEL = "gemini-3.8-flash"

# Reasoning effort: "low", "medium" or "high" (override with GEMINI_THINKING_LEVEL in .env).
# Measured on two songs (15 runs): timing precision was the same at every level (within ~0.5 s) while
# time and cost grew a lot: low ~5 s / $0.01, medium ~25 s / $0.03, high 1-3 min / up to $0.20.
# So low is the default. If a song with many repeated choruses or a very different arrangement gets
# confused, try GEMINI_THINKING_LEVEL=medium; high isn't worth it.
THINKING_LEVEL_DEFAULT = "low"
THINKING_LEVELS = ("low", "medium", "high")

def thinking_level() -> str:
    level = os.environ.get("GEMINI_THINKING_LEVEL", "").strip().lower()
    return level if level in THINKING_LEVELS else THINKING_LEVEL_DEFAULT

# Paid-tier price in USD per 1M tokens: (valid through this date, input, output). Output includes
# thinking tokens. Source: https://ai.google.dev/gemini-api/docs/pricing (introductory price until the end of 2026).
PRICE_SCHEDULE: List[Tuple[date, float, float]] = [
    (date(2026, 12, 31), 0.75, 3.75),
    (date.max, 1.50, 7.50),
]

# Env var holding the key (first one set wins)
API_KEY_VARS = ("GOOGLE_AI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY")

# Google sometimes answers "high demand" (503); retry a few times before giving up
BUSY_RETRIES = 3
BUSY_RETRY_DELAY_SECONDS = 8

# How long to wait for Google to finish processing the uploaded video
FILE_PROCESSING_TIMEOUT_SECONDS = 180

# Start times closer than this to the previous verse's start are treated as inconsistent
MIN_VERSE_GAP_SECONDS = 0.1

class GeminiTimingError(Exception):
    """Raised with a user-presentable message when automatic timing can't be completed."""

# ---------------------------------------------------------------- cost

def price_on(day: date) -> Tuple[Optional[date], float, float]:
    """(price valid through, input $/1M tokens, output $/1M tokens) in effect on a given day."""
    for valid_through, input_price, output_price in PRICE_SCHEDULE:
        if day <= valid_through:
            return (None if valid_through == date.max else valid_through), input_price, output_price
    raise AssertionError("PRICE_SCHEDULE must end with date.max")

def compute_cost(usage: Any, day: Optional[date] = None) -> Dict[str, Any]:
    """Cost of one request from Gemini's usage metadata."""
    valid_through, input_price, output_price = price_on(day or date.today())
    input_tokens = int(getattr(usage, "prompt_token_count", 0) or 0)
    # The answer's length is "candidates_token_count" in the API (named "response_token_count" in some SDK versions)
    answer_tokens = int(getattr(usage, "candidates_token_count", None) or getattr(usage, "response_token_count", 0) or 0)
    thinking_tokens = int(getattr(usage, "thoughts_token_count", 0) or 0)
    # Thinking is billed as output. Trust Gemini's own total when it reports more than the parts add up to.
    total_tokens = int(getattr(usage, "total_token_count", 0) or 0)
    output_tokens = max(answer_tokens + thinking_tokens, total_tokens - input_tokens)

    by_modality = {}
    for detail in getattr(usage, "prompt_tokens_details", None) or []:
        name = str(getattr(detail.modality, "name", detail.modality)).lower()
        by_modality[name] = by_modality.get(name, 0) + int(detail.token_count or 0)

    input_cost = input_tokens / 1_000_000 * input_price
    output_cost = output_tokens / 1_000_000 * output_price
    return {
        "model": GEMINI_MODEL,
        "input_tokens": input_tokens,
        "input_tokens_by_modality": by_modality,
        "output_tokens": output_tokens - thinking_tokens,  # the answer itself
        "thinking_tokens": thinking_tokens,
        "input_price_per_million": input_price,
        "output_price_per_million": output_price,
        "price_valid_through": valid_through.isoformat() if valid_through else None,
        "input_cost_usd": input_cost,
        "output_cost_usd": output_cost,
        "total_cost_usd": input_cost + output_cost,
    }

# ---------------------------------------------------------------- prompt & result parsing

_RESPONSE_SCHEMA = {
    "type": "object",
    "properties": {
        "verses": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "verse": {"type": "integer", "description": "The verse number, as given in the lyrics"},
                    "start": {"type": "string", "description": "Timestamp in the video (MM:SS.s, e.g. 01:13.4) when the first word is sung"},
                    "end": {"type": "string", "description": "Timestamp in the video (MM:SS.s, e.g. 01:19.0) when the last word ends"},
                },
                "required": ["verse", "start", "end"],
            },
        },
    },
    "required": ["verses"],
}

# [H:]M:SS[.s]
_TIMESTAMP = re.compile(r"^\s*(?:(\d+):)?(\d+):(\d{1,2}(?:[.,]\d+)?)\s*$")

def parse_timestamp(value: Any) -> Optional[float]:
    """Seconds from a timestamp like "1:13.4", "01:13", "1:02:03" (also a plain number of seconds); None if unusable."""
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        return float(value)
    if not isinstance(value, str):
        return None
    match = _TIMESTAMP.match(value)
    if match:
        hours, minutes, seconds = match.groups()
        seconds = float(seconds.replace(",", "."))
        if seconds >= 60:
            return None
        return int(hours or 0) * 3600 + int(minutes) * 60 + seconds
    try:
        return float(value.strip().replace(",", "."))
    except ValueError:
        return None

def format_timestamp(seconds: float) -> str:
    """Seconds as M:SS (H:MM:SS for long videos), rounded to the nearest second."""
    total = int(round(seconds))
    hours, rest = divmod(total, 3600)
    minutes, secs = divmod(rest, 60)
    return f"{hours}:{minutes:02d}:{secs:02d}" if hours else f"{minutes}:{secs:02d}"

def build_prompt(verses: List[str], title: str = "", artist: str = "", duration: float = 0.0) -> str:
    numbered = "\n\n".join(f"[{i}]\n{text.strip()}" for i, text in enumerate(verses, 1))
    about = " - ".join(part for part in (title.strip(), artist.strip()) if part)
    return (
        "You are given a video of a song and the song's lyrics, split into numbered verses.\n"
        "Listen to the audio and, for every verse, report when it is sung: the time its first word "
        "starts and the time its last word ends.\n\n"
        "Rules:\n"
        "- Give every time as a timestamp from the very start of the video in MM:SS.s format, with one "
        "digit after the seconds (for example 01:13.4 means 1 minute and 13.4 seconds).\n"
        + (f"- The video is {format_timestamp(duration)} long, so no timestamp may be later than that.\n" if duration > 0 else "")
        + 
        "- The verses are listed in the order they are sung. Start times must increase from one verse "
        "to the next, and a verse cannot start before the previous one ends.\n"
        "- If the same words are sung more than once in the song, each numbered verse below is one "
        "occurrence, in order. Match each verse to its own occurrence.\n"
        "- Ignore intros, instrumental breaks, spoken parts, and scenes that are not part of the song.\n"
        "- The singer may differ slightly from the written words; match by what is sung.\n"
        "- Return an entry for every verse number, even if you are unsure.\n\n"
        + (f"Song: {about}\n\n" if about else "")
        + "Lyrics:\n\n" + numbered
    )

def parse_timings(raw: str, verse_count: int, duration: float = 0.0) -> Tuple[List[Dict[str, Optional[float]]], List[str]]:
    """Turn the model's JSON into one {start_time, end_time} per verse (None when unusable).

    Returns (timings, warnings). Times that are missing, out of range or not increasing are dropped
    rather than guessed, so a bad answer never produces overlapping verses.
    """
    try:
        entries = json.loads(raw)["verses"]
    except (ValueError, KeyError, TypeError):
        entries = None
    if not isinstance(entries, list):
        raise GeminiTimingError("התשובה של Gemini לא הייתה בפורמט צפוי. נסה שוב.")

    by_number: Dict[int, Dict[str, Any]] = {}
    for entry in entries:
        try:
            by_number[int(entry["verse"])] = entry
        except (KeyError, TypeError, ValueError):
            continue

    timings: List[Dict[str, Optional[float]]] = []
    warnings: List[str] = []
    previous_start: Optional[float] = None
    for number in range(1, verse_count + 1):
        entry = by_number.get(number)
        timing: Dict[str, Optional[float]] = {"start_time": None, "end_time": None}
        timings.append(timing)
        try:
            start = parse_timestamp(entry["start"])
        except (KeyError, TypeError):
            start = None
        if start is None:
            not_played = isinstance(entry, dict) and entry.get("start") == ""  # the model says it isn't in the recording
            warnings.append(f"בית {number}: " + ("לא נמצא בהקלטה" if not_played else "לא התקבל תזמון"))
            continue
        start = round(start, 1)
        if start < 0 or (duration and start > duration + 1):
            warnings.append(f"בית {number}: תזמון מחוץ לאורך הסרטון ({format_timestamp(start)})")
            continue
        if previous_start is not None and start < previous_start + MIN_VERSE_GAP_SECONDS:
            warnings.append(f"בית {number}: תזמון לא עקבי עם הבית הקודם ({format_timestamp(start)})")
            continue
        timing["start_time"] = start
        previous_start = start
        try:
            end = parse_timestamp(entry["end"])
        except (KeyError, TypeError):
            end = None
        if end is not None and round(end, 1) > start:
            timing["end_time"] = round(end, 1)

    # An end that runs into the next timed verse is dropped; the video then ends it when the next starts
    timed = [t for t in timings if t["start_time"] is not None]
    for current, following in zip(timed, timed[1:]):
        if current["end_time"] is not None and current["end_time"] > following["start_time"] - MIN_VERSE_GAP_SECONDS:
            current["end_time"] = None
    return timings, warnings

# ---------------------------------------------------------------- Gemini call

def _api_key() -> str:
    for name in API_KEY_VARS:
        value = os.environ.get(name, "").strip()
        if value:
            return value
    raise GeminiTimingError(f"לא נמצא מפתח API של Gemini. הוסף את {API_KEY_VARS[0]} לקובץ .env והפעל מחדש את האפליקציה.")

def _retry_hint(detail: str) -> str:
    """"Please retry in 12h7m28.7s" from Google's quota error, as Hebrew text ("12 שעות ו-7 דקות"); "" if absent."""
    match = re.search(r"retry in (?:(\d+)h)?(?:(\d+)m)?(?:[\d.]+s)?", detail)
    if not match or not (match.group(1) or match.group(2)):
        return ""
    hours, minutes = int(match.group(1) or 0), int(match.group(2) or 0)
    parts = []
    if hours:
        parts.append(f"{hours} שעות")
    if minutes:
        parts.append(f"{minutes} דקות")
    return " ו-".join(parts)

def _friendly_api_error(error: Exception, key: str) -> GeminiTimingError:
    """Describe a Gemini API failure in Hebrew, without ever echoing the API key."""
    code = getattr(error, "code", None)
    detail = str(getattr(error, "message", "") or error).replace(key, "***")
    if code in (400, 401, 403) and "api key" in detail.lower():
        return GeminiTimingError("מפתח ה-API של Gemini לא תקין. בדוק את הערך ב-.env.")
    if code == 403:
        return GeminiTimingError(f"אין הרשאה לשימוש ב-Gemini עם המפתח הזה: {detail}")
    if code == 429:
        retry = _retry_hint(detail)
        again = f" אפשר לנסות שוב בעוד {retry}." if retry else " נסה שוב מאוחר יותר."
        limit = re.search(r"limit: (\d+)", detail)
        if "free_tier" in detail:
            quota = f" ({limit.group(1)} בקשות ביום למודל)" if limit else ""
            return GeminiTimingError(f"חרגת ממכסת הבקשות של החשבון החינמי ב-Google AI Studio{quota}.{again}")
        return GeminiTimingError(f"חריגה ממכסת השימוש של Gemini.{again}")
    if code in (500, 502, 503, 504):
        return GeminiTimingError("השירות של Gemini עמוס כרגע. נסה שוב בעוד רגע.")
    return GeminiTimingError(f"שגיאה מ-Gemini ({code}): {detail}")

def _generate_with_retries(client: genai.Client, contents: List[Any], schema: Dict[str, Any]) -> Any:
    """generate_content, retrying a few times when Google reports it is busy (HTTP 5xx)."""
    config = types.GenerateContentConfig(
        response_mime_type="application/json",
        response_json_schema=schema,
        thinking_config=types.ThinkingConfig(thinking_level=thinking_level()),
        automatic_function_calling=types.AutomaticFunctionCallingConfig(disable=True),
    )
    for attempt in range(BUSY_RETRIES + 1):
        try:
            return client.models.generate_content(model=GEMINI_MODEL, contents=contents, config=config)
        except genai_errors.ServerError:
            if attempt == BUSY_RETRIES:
                raise
            time.sleep(BUSY_RETRY_DELAY_SECONDS * (attempt + 1))

def _wait_until_active(client: genai.Client, uploaded: Any) -> Any:
    """Google processes an uploaded file before it can be used; poll until it is ready."""
    deadline = time.time() + FILE_PROCESSING_TIMEOUT_SECONDS
    while uploaded.state is not None and uploaded.state.name == "PROCESSING":
        if time.time() > deadline:
            raise GeminiTimingError("Google לא סיים לעבד את הקובץ בזמן. נסה שוב.")
        time.sleep(2)
        uploaded = client.files.get(name=uploaded.name)
    if uploaded.state is not None and uploaded.state.name == "FAILED":
        raise GeminiTimingError("Google לא הצליח לעבד את הקובץ.")
    return uploaded

def _run_gemini(verses: List[str], files: List[Path], build_contents: Callable[[List[Any]], List[Any]],
                schema: Dict[str, Any], duration: float) -> Dict[str, Any]:
    """Upload files to Google, ask Gemini, and turn its answer into verse timings plus the cost.

    build_contents receives the uploaded file handles (same order as files) and returns the request
    contents. Everything uploaded is deleted from Google afterwards.
    """
    key = _api_key()
    client = genai.Client(api_key=key)
    uploaded: List[Any] = []
    try:
        try:
            for path in files:
                uploaded.append(client.files.upload(file=str(path)))  # tracked at once, so it is deleted even if waiting fails
                uploaded[-1] = _wait_until_active(client, uploaded[-1])
            response = _generate_with_retries(client, build_contents(uploaded), schema)
        except genai_errors.APIError as e:
            raise _friendly_api_error(e, key)

        if not response.text:
            raise GeminiTimingError("Gemini לא החזיר תשובה (ייתכן שהתוכן נחסם). נסה שוב.")
        timings, warnings = parse_timings(response.text, len(verses), duration)
        return {
            "timings": timings,
            "warnings": warnings,
            "cost": compute_cost(response.usage_metadata),
        }
    finally:
        for handle in uploaded:
            try:
                client.files.delete(name=handle.name)
            except Exception:
                pass

def _clean_verses(verses: List[str]) -> List[str]:
    cleaned = [v for v in (v.strip() for v in verses) if v]
    if not cleaned:
        raise GeminiTimingError("אין בתים לתזמון. הוסף מילים קודם.")
    return cleaned

def time_verses_from_youtube(youtube_url: str, verses: List[str], title: str = "", artist: str = "") -> Dict[str, Any]:
    """Time each verse of a song against its YouTube video using Gemini.

    Downloads a small copy of the video, uploads it to Google, asks Gemini when each verse is sung,
    and returns {"timings": [{start_time, end_time}...], "warnings": [...], "cost": {...}, ...}.
    """
    verses = _clean_verses(verses)
    _api_key()  # fail early, before downloading anything

    work_dir = Path(tempfile.mkdtemp(prefix="singalong-ai-"))
    try:
        try:
            video_path, info = youtube.download_video(youtube_url, work_dir)
        except AudioDownloadError as e:
            raise GeminiTimingError(str(e))
        duration = float(info.get("duration") or 0)
        result = _run_gemini(
            verses, [video_path],
            lambda files: [files[0], build_prompt(verses, title, artist, duration)],
            _RESPONSE_SCHEMA, duration)
        result["video_seconds"] = duration
        return result
    finally:
        shutil.rmtree(work_dir, ignore_errors=True)

# ---------------------------------------------------------------- karaoke + singer version

_SINGER_RESPONSE_SCHEMA = {
    "type": "object",
    "properties": {
        "verses": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "verse": {"type": "integer", "description": "The verse number, as given in the lyrics"},
                    "singer_start": {"type": "string", "description": "Timestamp in the SINGER recording (MM:SS.s) where the verse's first word is sung"},
                    "start": {"type": "string", "description": "Timestamp in the KARAOKE recording (MM:SS.s) where the verse starts"},
                    "end": {"type": "string", "description": "Timestamp in the KARAOKE recording (MM:SS.s) where the verse ends"},
                },
                "required": ["verse", "singer_start", "start", "end"],
            },
        },
    },
    "required": ["verses"],
}

def build_singer_prompt(verses: List[str], title: str = "", artist: str = "",
                        karaoke_seconds: float = 0.0, singer_seconds: float = 0.0) -> str:
    numbered = "\n\n".join(f"[{i}]\n{text.strip()}" for i, text in enumerate(verses, 1))
    about = " - ".join(part for part in (title.strip(), artist.strip()) if part)
    karaoke_length = f" It is {format_timestamp(karaoke_seconds)} long." if karaoke_seconds > 0 else ""
    singer_length = f" It is {format_timestamp(singer_seconds)} long." if singer_seconds > 0 else ""
    return (
        "You are given two audio recordings of the same song, and the song's lyrics split into numbered verses.\n"
        f"- Recording 1 is the KARAOKE version: the backing track without a lead singer.{karaoke_length}\n"
        f"- Recording 2 is a version WITH A SINGER who sings the lyrics.{singer_length}\n\n"
        "Your task is to time every verse in the KARAOKE recording: the moment in the karaoke recording "
        "when the verse should start being sung, and the moment it ends. Use the singer version to hear "
        "when each verse is sung, then map those moments onto the karaoke recording.\n\n"
        "Rules:\n"
        "- The two recordings can differ in the length of the intro and outro, tempo, key, arrangement, "
        "or even structure (for example a skipped or extra repeat). Do not assume that the same timestamp "
        "means the same moment in both. Align them by the music: match sections such as intro, verses, "
        "choruses, bridges and instrumental breaks.\n"
        "- Give every time as a timestamp from the very start of that recording in MM:SS.s format, with "
        "one digit after the seconds (for example 01:13.4 means 1 minute and 13.4 seconds). "
        "\"singer_start\" is a time in the singer recording; \"start\" and \"end\" are times in the "
        "karaoke recording and must not be later than its length.\n"
        "- The verses are listed in the order they are sung. Start times must increase from one verse to "
        "the next, and a verse cannot start before the previous one ends.\n"
        "- If the same words are sung more than once in the song, each numbered verse below is one "
        "occurrence, in order. Match each verse to its own occurrence.\n"
        "- Ignore intros, instrumental breaks and spoken parts; do not assign them to a verse.\n"
        "- Only the verses listed below matter. The singer version may sing extra verses that are not "
        "listed (skip those), and the karaoke recording may leave out one of the listed verses (for "
        "example playing an instrumental break instead). Use an empty string for \"start\" and \"end\" "
        "only if a listed verse is really not played in the karaoke recording.\n"
        "- Return an entry for every verse number, even if you are unsure.\n\n"
        + (f"Song: {about}\n\n" if about else "")
        + "Lyrics:\n\n" + numbered
    )

def time_verses_from_singer_version(singer_url: str, karaoke_audio: Path, verses: List[str],
                                    title: str = "", artist: str = "") -> Dict[str, Any]:
    """Time the verses in a karaoke (backing track) recording, using a version with a singer as a guide.

    The singer version is downloaded from singer_url and converted to MP3; both recordings are
    uploaded to Google and Gemini maps each verse from the singer version onto the karaoke one.
    """
    verses = _clean_verses(verses)
    _api_key()  # fail early, before downloading anything
    if not karaoke_audio.is_file():
        raise GeminiTimingError("לא נמצא קובץ השמע של השיר (גרסת הקריוקי).")

    work_dir = Path(tempfile.mkdtemp(prefix="singalong-ai-"))
    try:
        try:
            singer_bytes, singer_info = youtube.download_audio_mp3(singer_url)
        except AudioDownloadError as e:
            raise GeminiTimingError(str(e))
        singer_path = work_dir / "singer.mp3"
        singer_path.write_bytes(singer_bytes)
        singer_seconds = float(singer_info.get("duration") or 0) or youtube.ffprobe_duration(singer_path)
        karaoke_seconds = youtube.ffprobe_duration(karaoke_audio)

        result = _run_gemini(
            verses, [karaoke_audio, singer_path],
            lambda files: [
                "Recording 1 - the KARAOKE version (backing track without the lead singer):", files[0],
                "Recording 2 - a version WITH A SINGER:", files[1],
                build_singer_prompt(verses, title, artist, karaoke_seconds, singer_seconds),
            ],
            _SINGER_RESPONSE_SCHEMA, karaoke_seconds)
        result["karaoke_seconds"] = karaoke_seconds
        result["singer_seconds"] = singer_seconds
        return result
    finally:
        shutil.rmtree(work_dir, ignore_errors=True)
