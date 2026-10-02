import re
import shutil
import subprocess
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

def ffprobe_duration(path: Path) -> float:
    """Length in seconds of an audio or video file (0.0 if it can't be read)."""
    probe = shutil.which("ffprobe", path=_ffmpeg_dir()) or shutil.which("ffprobe")
    if not probe:
        return 0.0
    try:
        out = subprocess.run(
            [probe, "-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", str(path)],
            capture_output=True, text=True, timeout=30)
        return float(out.stdout.strip())
    except (ValueError, OSError, subprocess.SubprocessError):
        return 0.0

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

def normalize_url(url: str) -> str:
    """Return a clean http(s) link ("www.youtube.com/..." gets https://), or raise AudioDownloadError."""
    url = (url or "").strip()
    if url and "://" not in url:
        url = "https://" + url  # allow pasting "www.youtube.com/watch?v=..."
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https") or "." not in parsed.netloc or re.search(r"\s", url):
        raise AudioDownloadError("הקישור אינו תקין. הדבק כתובת מלאה שמתחילה ב-https://")
    return url

def _run_ytdlp(url: str, out_dir: Path, basename: str, player_clients: Optional[List[str]],
               extra_opts: Dict[str, Any]) -> Dict[str, Any]:
    """One yt-dlp attempt writing out_dir/<basename>.<ext>; returns yt-dlp's info dict."""
    runtimes = _js_runtimes()
    opts = {
        "outtmpl": str(out_dir / f"{basename}.%(ext)s"),
        "noplaylist": True,
        "quiet": True,
        "noprogress": True,
        "no_warnings": True,
        "ffmpeg_location": _ffmpeg_dir(),
        "match_filter": match_filter_func(f"duration <= {MAX_DURATION_SECONDS}"),
        **extra_opts,
    }
    if runtimes:
        opts["js_runtimes"] = runtimes
    if player_clients:
        opts["extractor_args"] = {"youtube": {"player_client": player_clients}}
    with yt_dlp.YoutubeDL(opts) as ydl:
        return ydl.extract_info(url, download=True)

def _download_with_retries(url: str, work_dir: Path, basename: str,
                           extra_opts: Dict[str, Any]) -> Tuple[Path, Dict[str, Any]]:
    """Run yt-dlp, retrying through other YouTube clients when a download is refused.

    Each attempt gets its own subfolder of work_dir. Returns (that folder, yt-dlp's info dict).
    """
    last_error = ""
    for attempt, clients in enumerate(_PLAYER_CLIENT_ATTEMPTS):
        out_dir = work_dir / f"attempt{attempt}"
        out_dir.mkdir()
        try:
            info = _run_ytdlp(url, out_dir, basename, clients, extra_opts)
        except Exception as e:  # yt-dlp raises several unrelated error types (download, network, extractor)
            last_error = _clean_message(e)
            if any(marker in last_error.lower() for marker in _RETRYABLE):
                continue
            raise AudioDownloadError(f"ההורדה נכשלה: {last_error}") from e
        if not info:
            raise AudioDownloadError(
                f"לא ניתן להוריד את הסרטון (ייתכן שהוא ארוך מ-{MAX_DURATION_SECONDS // 60} דקות)")
        return out_dir, info

    raise AudioDownloadError(
        f"ההורדה נכשלה: {last_error} (יוטיוב חסם את ההורדה. נסה שוב בעוד רגע, "
        "או עדכן: pip install -U yt-dlp[default])")

def download_audio_mp3(url: str) -> Tuple[bytes, Dict[str, Any]]:
    """Download the audio of a video link and convert it to MP3. Returns (mp3 bytes, info)."""
    url = normalize_url(url)
    with tempfile.TemporaryDirectory() as tmp:
        out_dir, info = _download_with_retries(url, Path(tmp), "audio", {
            "format": "bestaudio/best",
            "postprocessors": [{
                "key": "FFmpegExtractAudio",
                "preferredcodec": "mp3",
                "preferredquality": "192",
            }],
        })
        mp3_files = list(out_dir.glob("audio.mp3"))
        if not mp3_files:
            raise AudioDownloadError(
                f"לא ניתן להוריד את הסרטון (ייתכן שהוא ארוך מ-{MAX_DURATION_SECONDS // 60} דקות)")
        return mp3_files[0].read_bytes(), {
            "title": info.get("track") or info.get("title") or "",
            "artist": info.get("artist") or "",
            "duration": info.get("duration") or 0,
        }

# Small video with its audio track: Gemini counts video tokens per second regardless of resolution,
# so a low resolution only saves download/upload time. Prefer MP4 (H.264 + AAC), which Gemini accepts.
_VIDEO_FORMAT = ("bv*[height<=360][vcodec^=avc1]+ba[ext=m4a]/bv*[height<=360][ext=mp4]+ba[ext=m4a]/"
                 "b[height<=360][ext=mp4]/bv*[height<=360]+ba/b[height<=360]/w")

def download_video(url: str, work_dir: Path) -> Tuple[Path, Dict[str, Any]]:
    """Download a small version of the video (with its audio) inside work_dir.

    Returns (video file, yt-dlp info). The caller owns work_dir and removes it afterwards.
    """
    url = normalize_url(url)
    out_dir, info = _download_with_retries(url, work_dir, "video", {
        "format": _VIDEO_FORMAT,
        "merge_output_format": "mp4",
    })
    files = [p for p in out_dir.glob("video.*") if p.is_file()]
    if not files:
        raise AudioDownloadError("הורדת הסרטון נכשלה: לא נוצר קובץ וידאו")
    return files[0], info
