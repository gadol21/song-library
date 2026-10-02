import json
import os
import shutil
import tempfile
import time
from datetime import date
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

from google import genai
from google.genai import errors as genai_errors
from google.genai import types

from app import youtube
from app.youtube import AudioDownloadError

GEMINI_MODEL = "gemini-3.8-flash"

# Reasoning effort. Timing is mostly listening, so a low level keeps the cost down.
THINKING_LEVEL = "low"

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
                    "start": {"type": "number", "description": "Seconds from the start of the video when the first word is sung"},
                    "end": {"type": "number", "description": "Seconds from the start of the video when the last word ends"},
                },
                "required": ["verse", "start", "end"],
            },
        },
    },
    "required": ["verses"],
}

def build_prompt(verses: List[str], title: str = "", artist: str = "") -> str:
    numbered = "\n\n".join(f"[{i}]\n{text.strip()}" for i, text in enumerate(verses, 1))
    about = " - ".join(part for part in (title.strip(), artist.strip()) if part)
    return (
        "You are given a video of a song and the song's lyrics, split into numbered verses.\n"
        "Listen to the audio and, for every verse, report when it is sung: the time its first word "
        "starts and the time its last word ends.\n\n"
        "Rules:\n"
        "- Give all times in seconds from the very start of the video, as decimal numbers with one "
        "digit after the point (for example 73.4). Do not use minutes:seconds.\n"
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
            start = round(float(entry["start"]), 1)
        except (KeyError, TypeError, ValueError):
            warnings.append(f"בית {number}: לא התקבל תזמון")
            continue
        if start < 0 or (duration and start > duration + 1):
            warnings.append(f"בית {number}: תזמון מחוץ לאורך הסרטון ({start} שנ')")
            continue
        if previous_start is not None and start < previous_start + MIN_VERSE_GAP_SECONDS:
            warnings.append(f"בית {number}: תזמון לא עקבי עם הבית הקודם ({start} שנ')")
            continue
        timing["start_time"] = start
        previous_start = start
        try:
            end = round(float(entry["end"]), 1)
            if end > start:
                timing["end_time"] = end
        except (KeyError, TypeError, ValueError):
            pass

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

def _friendly_api_error(error: Exception, key: str) -> GeminiTimingError:
    """Describe a Gemini API failure in Hebrew, without ever echoing the API key."""
    code = getattr(error, "code", None)
    detail = str(getattr(error, "message", "") or error).replace(key, "***")
    if code in (400, 401, 403) and "api key" in detail.lower():
        return GeminiTimingError("מפתח ה-API של Gemini לא תקין. בדוק את הערך ב-.env.")
    if code == 403:
        return GeminiTimingError(f"אין הרשאה לשימוש ב-Gemini עם המפתח הזה: {detail}")
    if code == 429:
        return GeminiTimingError("חריגה ממכסת השימוש של Gemini (או יותר מדי בקשות). נסה שוב עוד כמה דקות.")
    if code in (500, 502, 503, 504):
        return GeminiTimingError("השירות של Gemini עמוס כרגע. נסה שוב בעוד רגע.")
    return GeminiTimingError(f"שגיאה מ-Gemini ({code}): {detail}")

def _generate_with_retries(client: genai.Client, contents: List[Any]) -> Any:
    """generate_content, retrying a few times when Google reports it is busy (HTTP 5xx)."""
    config = types.GenerateContentConfig(
        response_mime_type="application/json",
        response_json_schema=_RESPONSE_SCHEMA,
        thinking_config=types.ThinkingConfig(thinking_level=THINKING_LEVEL),
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
    """Google processes an uploaded video before it can be used; poll until it is ready."""
    deadline = time.time() + FILE_PROCESSING_TIMEOUT_SECONDS
    while uploaded.state is not None and uploaded.state.name == "PROCESSING":
        if time.time() > deadline:
            raise GeminiTimingError("Google לא סיים לעבד את הסרטון בזמן. נסה שוב.")
        time.sleep(2)
        uploaded = client.files.get(name=uploaded.name)
    if uploaded.state is not None and uploaded.state.name == "FAILED":
        raise GeminiTimingError("Google לא הצליח לעבד את קובץ הסרטון.")
    return uploaded

def time_verses_from_youtube(youtube_url: str, verses: List[str], title: str = "", artist: str = "") -> Dict[str, Any]:
    """Time each verse of a song against its YouTube video using Gemini.

    Downloads a small copy of the video, uploads it to Google, asks Gemini when each verse is sung,
    and returns {"timings": [{start_time, end_time}...], "warnings": [...], "cost": {...}, ...}.
    """
    verses = [v for v in (v.strip() for v in verses) if v]
    if not verses:
        raise GeminiTimingError("אין בתים לתזמון. הוסף מילים קודם.")
    key = _api_key()
    client = genai.Client(api_key=key)

    work_dir = Path(tempfile.mkdtemp(prefix="singalong-ai-"))
    uploaded = None
    try:
        try:
            video_path, info = youtube.download_video(youtube_url, work_dir)
        except AudioDownloadError as e:
            raise GeminiTimingError(str(e))
        duration = float(info.get("duration") or 0)

        try:
            uploaded = client.files.upload(file=str(video_path))
            uploaded = _wait_until_active(client, uploaded)
            response = _generate_with_retries(client, [uploaded, build_prompt(verses, title, artist)])
        except genai_errors.APIError as e:
            raise _friendly_api_error(e, key)

        if not response.text:
            raise GeminiTimingError("Gemini לא החזיר תשובה (ייתכן שהתוכן נחסם). נסה שוב.")
        timings, warnings = parse_timings(response.text, len(verses), duration)
        return {
            "timings": timings,
            "warnings": warnings,
            "video_seconds": duration,
            "cost": compute_cost(response.usage_metadata),
        }
    finally:
        # Don't leave the video behind: delete the Google copy and the local files
        if uploaded is not None:
            try:
                client.files.delete(name=uploaded.name)
            except Exception:
                pass
        shutil.rmtree(work_dir, ignore_errors=True)
