import re
import shutil
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import urlparse

import yt_dlp
from yt_dlp.utils import match_filter_func

from app.database import BASE_DIR

# Songs are never this long; the cap also stops accidental livestream/hour-long mix downloads
MAX_DURATION_SECONDS = 20 * 60

_ANSI_CODES = re.compile(r"\x1b\[[0-9;]*m")

# YouTube serves audio through several "clients", and yt-dlp picks them per request. Some of
# them hand out stream URLs that are refused with HTTP 403, so a failed download is retried
# through these alternatives (None = yt-dlp's own choice).
_PLAYER_CLIENT_ATTEMPTS: List[Optional[List[str]]] = [
    None,
    ["default", "-android_vr"],
    ["mweb"],
]

# Errors worth retrying with another client; anything else (private/removed video...) fails right away
_RETRYABLE = ("403", "forbidden", "requested format is not available", "page needs to be reloaded")

class AudioDownloadError(Exception):
    """Raised with a user-presentable message when a link can't be turned into an MP3."""

def _ffmpeg_dir() -> str:
    """Folder holding ffmpeg: the one on PATH (run.bat puts the bundled one first), else the bundled copy."""
    found = shutil.which("ffmpeg")
    if found:
        return str(Path(found).parent)
    return str(BASE_DIR / "ffmpeg" / "bin")

def _js_runtimes() -> Dict[str, Dict[str, str]]:
    """JavaScript runtimes found on this machine (YouTube needs one for its normal streams).

    yt-dlp only auto-detects Deno, so tell it about the others explicitly.
    """
    found = {}
    for name in ("deno", "node", "bun", "quickjs"):
        path = shutil.which(name)
        if path:
            found[name] = {"path": path}
    return found

def _clean_message(error: Exception) -> str:
    """yt-dlp colors its messages for terminals and prefixes them with "ERROR: "."""
    return _ANSI_CODES.sub("", str(error)).replace("ERROR: ", "").strip()

def _download(url: str, out_dir: Path, player_clients: Optional[List[str]]) -> Dict[str, Any]:
    """One download attempt into out_dir/audio.mp3; returns yt-dlp's info dict."""
    runtimes = _js_runtimes()
    opts = {
        "format": "bestaudio/best",
        "outtmpl": str(out_dir / "audio.%(ext)s"),
        "noplaylist": True,
        "quiet": True,
        "noprogress": True,
        "no_warnings": True,
        "ffmpeg_location": _ffmpeg_dir(),
        "match_filter": match_filter_func(f"duration <= {MAX_DURATION_SECONDS}"),
        "postprocessors": [{
            "key": "FFmpegExtractAudio",
            "preferredcodec": "mp3",
            "preferredquality": "192",
        }],
    }
    if runtimes:
        opts["js_runtimes"] = runtimes
    if player_clients:
        opts["extractor_args"] = {"youtube": {"player_client": player_clients}}
    with yt_dlp.YoutubeDL(opts) as ydl:
        return ydl.extract_info(url, download=True)

def download_audio_mp3(url: str) -> Tuple[bytes, Dict[str, Any]]:
    """Download the audio of a video link and convert it to MP3. Returns (mp3 bytes, info)."""
    url = (url or "").strip()
    if url and "://" not in url:
        url = "https://" + url  # allow pasting "www.youtube.com/watch?v=..."
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https") or "." not in parsed.netloc or re.search(r"\s", url):
        raise AudioDownloadError("הקישור אינו תקין. הדבק כתובת מלאה שמתחילה ב-https://")

    last_error = ""
    with tempfile.TemporaryDirectory() as tmp:
        for attempt, clients in enumerate(_PLAYER_CLIENT_ATTEMPTS):
            out_dir = Path(tmp) / f"attempt{attempt}"
            out_dir.mkdir()
            try:
                info = _download(url, out_dir, clients)
            except Exception as e:  # yt-dlp raises several unrelated error types (download, network, extractor)
                last_error = _clean_message(e)
                if any(marker in last_error.lower() for marker in _RETRYABLE):
                    continue
                raise AudioDownloadError(f"ההורדה נכשלה: {last_error}") from e

            mp3_files = list(out_dir.glob("audio.mp3"))
            if not info or not mp3_files:
                raise AudioDownloadError(
                    f"לא ניתן להוריד את הסרטון (ייתכן שהוא ארוך מ-{MAX_DURATION_SECONDS // 60} דקות)")

            return mp3_files[0].read_bytes(), {
                "title": info.get("track") or info.get("title") or "",
                "artist": info.get("artist") or "",
                "duration": info.get("duration") or 0,
            }

    raise AudioDownloadError(
        f"ההורדה נכשלה: {last_error} (יוטיוב חסם את ההורדה. נסה שוב בעוד רגע, "
        "או עדכן: pip install -U yt-dlp[default])")
