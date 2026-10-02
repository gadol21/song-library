import os
import json
import re
import shutil
import time
from pathlib import Path
from typing import List, Dict, Optional, Any

from dotenv import load_dotenv

BASE_DIR = Path(__file__).resolve().parent.parent

# Optional .env in the project folder can relocate the data (e.g. onto a backed-up drive)
load_dotenv(BASE_DIR / ".env")

def _env_dir(name: str, default: str) -> Path:
    """Folder from an environment variable; ~ is expanded and relative paths are relative to the project."""
    raw = os.environ.get(name, "").strip().strip('"').strip("'") or default
    path = Path(raw).expanduser()
    return path if path.is_absolute() else (BASE_DIR / path).resolve()

# DATA_DIR holds songs (audio + lyrics) and performances; EXPORTS_DIR holds generated decks and videos
DATA_DIR = _env_dir("SINGALONG_DATA_DIR", "data")
SONGS_DIR = DATA_DIR / "songs"
PERFORMANCES_DIR = DATA_DIR / "performances"
EXPORTS_DIR = _env_dir("SINGALONG_EXPORTS_DIR", "exports")
PRESENTATIONS_DIR = EXPORTS_DIR / "presentations"
VIDEOS_DIR = EXPORTS_DIR / "videos"

def init_db():
    """Ensure all required directories exist."""
    SONGS_DIR.mkdir(parents=True, exist_ok=True)
    PERFORMANCES_DIR.mkdir(parents=True, exist_ok=True)
    PRESENTATIONS_DIR.mkdir(parents=True, exist_ok=True)
    VIDEOS_DIR.mkdir(parents=True, exist_ok=True)

def detect_language(text: str) -> str:
    """Detect if text contains Hebrew characters."""
    if not text:
        return "en"
    # Hebrew Unicode range: \u0590 - \u05FF
    hebrew_count = len(re.findall(r'[\u0590-\u05FF]', text))
    return "he" if hebrew_count > 0 else "en"

def slugify(text: str) -> str:
    """Generate a clean, filesystem-safe ID from text (Hebrew and English friendly)."""
    # Replace whitespace and invalid filesystem characters with hyphens
    clean = re.sub(r'[^\w\u0590-\u05FF\s-]', '', text).strip()
    slug = re.sub(r'[\s_]+', '-', clean).lower()
    if not slug:
        slug = f"song-{int(time.time())}"
    return slug

def format_lrc_time(seconds: float) -> str:
    """Convert float seconds to [mm:ss.xx] LRC format."""
    mins = int(seconds // 60)
    secs = seconds % 60
    return f"[{mins:02d}:{secs:05.2f}]"

def _verse_start(verse: Dict[str, Any]) -> float:
    """Start time of a verse; untimed verses (missing or null) count as 0."""
    start = verse.get("start_time")
    return float(start) if start is not None else 0.0

def _verse_end(verse: Dict[str, Any]) -> float:
    """End time of a verse; falls back to start + 5s when missing or null."""
    end = verse.get("end_time")
    return float(end) if end is not None else _verse_start(verse) + 5.0

def generate_lrc(verses: List[Dict[str, Any]], title: str = "", artist: str = "") -> str:
    """Convert verses list into standard LRC text."""
    lines = [
        f"[ti:{title}]",
        f"[ar:{artist}]",
        "[by:Sing-Along Studio]",
        ""
    ]
    for verse in sorted(verses, key=_verse_start):
        start = _verse_start(verse)
        # Handle multi-line verses by adding timestamp to each line or stanza
        verse_lines = verse.get("text", "").strip().split("\n")
        time_tag = format_lrc_time(start)
        for idx, line in enumerate(verse_lines):
            line_str = line.strip()
            if line_str:
                if idx == 0:
                    lines.append(f"{time_tag} {line_str}")
                else:
                    lines.append(line_str)
        lines.append("")
    return "\n".join(lines)

def list_songs(search: Optional[str] = None, language: Optional[str] = None) -> List[Dict[str, Any]]:
    """List all songs stored in filesystem."""
    init_db()
    songs = []
    if not SONGS_DIR.exists():
        return songs

    for song_folder in SONGS_DIR.iterdir():
        if song_folder.is_dir():
            json_file = song_folder / "song.json"
            if json_file.exists():
                try:
                    with open(json_file, "r", encoding="utf-8") as f:
                        data = json.load(f)
                    # Check for audio file
                    audio_files = list(song_folder.glob("audio.*"))
                    data["has_audio"] = len(audio_files) > 0
                    if audio_files:
                        data["audio_filename"] = audio_files[0].name

                    # Filter
                    if language and language != "all" and data.get("language") != language:
                        continue
                    if search:
                        s_lower = search.lower()
                        title = data.get("title", "").lower()
                        artist = data.get("artist", "").lower()
                        if s_lower not in title and s_lower not in artist:
                            continue

                    songs.append(data)
                except Exception as e:
                    print(f"Error reading {json_file}: {e}")

    # Sort alphabetically by title
    songs.sort(key=lambda s: s.get("title", "").lower())
    return songs

def get_song(song_id: str) -> Optional[Dict[str, Any]]:
    """Retrieve song by ID."""
    init_db()
    folder = SONGS_DIR / song_id
    json_file = folder / "song.json"
    if not json_file.exists():
        return None
    try:
        with open(json_file, "r", encoding="utf-8") as f:
            data = json.load(f)
        audio_files = list(folder.glob("audio.*"))
        data["has_audio"] = len(audio_files) > 0
        if audio_files:
            data["audio_filename"] = audio_files[0].name
            data["audio_path"] = str(audio_files[0])
        return data
    except Exception as e:
        print(f"Error loading song {song_id}: {e}")
        return None

def save_song(song_data: Dict[str, Any], audio_bytes: Optional[bytes] = None, audio_ext: str = ".mp3") -> Dict[str, Any]:
    """Save or update song in filesystem."""
    init_db()
    song_id = song_data.get("id") or slugify(song_data.get("title", "untitled"))
    song_data["id"] = song_id

    # Auto detect language if not explicitly provided
    if not song_data.get("language"):
        all_text = song_data.get("title", "") + " " + " ".join([v.get("text", "") for v in song_data.get("verses", [])])
        song_data["language"] = detect_language(all_text)

    # Calculate duration if verses provided and duration not set
    verses = song_data.get("verses", [])
    if verses:
        max_end = max([_verse_end(v) for v in verses], default=0.0)
        if not song_data.get("duration") or song_data.get("duration") < max_end:
            song_data["duration"] = round(max_end + 3.0, 1)

    folder = SONGS_DIR / song_id
    folder.mkdir(parents=True, exist_ok=True)

    # Save audio if provided
    if audio_bytes:
        # Remove any existing audio.* files first
        for old_audio in folder.glob("audio.*"):
            old_audio.unlink()
        audio_target = folder / f"audio{audio_ext}"
        with open(audio_target, "wb") as f:
            f.write(audio_bytes)
        song_data["has_audio"] = True
        song_data["audio_filename"] = audio_target.name
        song_data["audio_path"] = str(audio_target)

    # Save song.json
    song_data["updated_at"] = time.time()
    json_path = folder / "song.json"
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(song_data, f, ensure_ascii=False, indent=2)

    # Save lyrics.lrc
    lrc_content = generate_lrc(verses, song_data.get("title", ""), song_data.get("artist", ""))
    lrc_path = folder / "lyrics.lrc"
    with open(lrc_path, "w", encoding="utf-8") as f:
        f.write(lrc_content)

    return song_data

def delete_song(song_id: str) -> bool:
    """Delete a song folder."""
    folder = SONGS_DIR / song_id
    if folder.exists() and folder.is_dir():
        shutil.rmtree(folder)
        return True
    return False

def list_performances() -> List[Dict[str, Any]]:
    """List all performances."""
    init_db()
    performances = []
    for file in PERFORMANCES_DIR.glob("*.json"):
        try:
            with open(file, "r", encoding="utf-8") as f:
                data = json.load(f)
            performances.append(data)
        except Exception as e:
            print(f"Error loading performance {file}: {e}")
    performances.sort(key=lambda p: p.get("created_at", 0), reverse=True)
    return performances

def get_performance(perf_id: str) -> Optional[Dict[str, Any]]:
    """Get single performance with resolved song details."""
    init_db()
    file = PERFORMANCES_DIR / f"{perf_id}.json"
    if not file.exists():
        return None
    try:
        with open(file, "r", encoding="utf-8") as f:
            data = json.load(f)
        # Populate full song details for each song_id in performance
        detailed_songs = []
        total_duration = 0.0
        for sid in data.get("song_ids", []):
            song = get_song(sid)
            if song:
                detailed_songs.append(song)
                total_duration += song.get("duration", 0.0)
        data["songs"] = detailed_songs
        data["total_duration"] = total_duration
        return data
    except Exception as e:
        print(f"Error loading performance {perf_id}: {e}")
        return None

def save_performance(perf_data: Dict[str, Any]) -> Dict[str, Any]:
    """Save or update performance."""
    init_db()
    perf_id = perf_data.get("id") or slugify(perf_data.get("title", "performance"))
    perf_data["id"] = perf_id
    if not perf_data.get("created_at"):
        perf_data["created_at"] = time.time()
    perf_data["updated_at"] = time.time()

    file = PERFORMANCES_DIR / f"{perf_id}.json"
    with open(file, "w", encoding="utf-8") as f:
        json.dump(perf_data, f, ensure_ascii=False, indent=2)

    return get_performance(perf_id) or perf_data

def delete_performance(perf_id: str) -> bool:
    """Delete a performance file."""
    file = PERFORMANCES_DIR / f"{perf_id}.json"
    if file.exists():
        file.unlink()
        return True
    return False
