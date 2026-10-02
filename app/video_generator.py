import os
import subprocess
import time
from pathlib import Path
from typing import Dict, Any, List, Optional

from app.database import VIDEOS_DIR, detect_language

def escape_filter_path(path: Path) -> str:
    """Quote a file path for use inside an FFmpeg filter argument (handles Windows drive colons/backslashes)."""
    p = Path(path).as_posix().replace("'", r"'\''").replace(":", r"\:")
    return f"'{p}'"

def format_ass_time(seconds: float) -> str:
    """Format seconds into ASS timestamp format H:MM:SS.cs"""
    h = int(seconds // 3600)
    m = int((seconds % 3600) // 60)
    s = int(seconds % 60)
    cs = int(round((seconds - int(seconds)) * 100))
    if cs >= 100:
        s += 1
        cs -= 100
    return f"{h}:{m:02d}:{s:02d}.{cs:02d}"

def generate_ass_subtitles(song: Dict[str, Any], total_duration: float) -> str:
    """Generate Advanced SubStation Alpha (.ass) subtitle file for karaoke rendering."""
    title = song.get("title", "")
    artist = song.get("artist", "")
    verses = song.get("verses", [])
    
    # Sort verses by start time
    sorted_verses = sorted(verses, key=lambda v: v.get("start_time") or 0.0)

    header_text = f"{title} - {artist}" if artist else title

    # ASS Header & Styles
    ass_lines = [
        "[Script Info]",
        "Title: Karaoke Video",
        "ScriptType: v4.00+",
        "WrapStyle: 0",
        "ScaledBorderAndShadow: yes",
        "PlayResX: 1280",
        "PlayResY: 720",
        "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        # Title at the top center
        r"Style: Header,Arial,30,&H0038BDF8,&H00000000,&H00000000,&H80000000,1,0,0,0,100,100,0,0,1,2,1,8,40,40,35,1",
        # Active verse: centered, large, vibrant white with subtle gold outline
        r"Style: ActiveVerse,Arial,42,&H00FFFFFF,&H00000000,&H00000000,&HA0000000,1,0,0,0,100,100,0,0,1,3,2,5,70,70,40,1",
        # Next verse preview: bottom, smaller, muted slate
        r"Style: NextVerse,Arial,26,&H0094A3B8,&H00000000,&H00000000,&HA0000000,0,0,0,0,100,100,0,0,1,2,1,2,70,70,55,1",
        "",
        "[Events]",
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        # Persistent title at top for the whole song
        f"Dialogue: 0,0:00:00.00,{format_ass_time(total_duration)},Header,,0,0,0,,{header_text}"
    ]

    for idx, v in enumerate(sorted_verses):
        start = max(0.0, float(v.get("start_time") or 0.0))
        
        # Calculate end time: until next verse starts, or end_time, or start + 6s
        if v.get("end_time") and float(v.get("end_time")) > start:
            end = float(v.get("end_time"))
        elif idx + 1 < len(sorted_verses):
            end = max(start + 1.0, float(sorted_verses[idx + 1].get("start_time") or start + 5.0) - 0.2)
        else:
            end = min(total_duration, start + 6.0)

        # Format verse text: replace newlines with ASS \N
        raw_text = v.get("text", "").strip()
        lines = [line.strip() for line in raw_text.split("\n") if line.strip()]
        formatted_text = r"\N".join(lines)

        start_str = format_ass_time(start)
        end_str = format_ass_time(end)

        # Active verse
        ass_lines.append(f"Dialogue: 1,{start_str},{end_str},ActiveVerse,,0,0,0,,{formatted_text}")

        # Next verse preview (if there is an upcoming verse)
        if idx + 1 < len(sorted_verses):
            next_v = sorted_verses[idx + 1]
            next_raw = next_v.get("text", "").strip()
            next_lines = [l.strip() for l in next_raw.split("\n") if l.strip()]
            first_line = next_lines[0] if next_lines else ""
            lang = detect_language(first_line)
            label = "הבא: " if lang == "he" else "Next: "
            ass_lines.append(f"Dialogue: 0,{start_str},{end_str},NextVerse,,0,0,0,,{label}{first_line}")

    return "\n".join(ass_lines)

def generate_karaoke_video(song: Dict[str, Any], filename_prefix: str = "karaoke") -> str:
    """Render a 720p HD MP4 karaoke video for a song using FFmpeg."""
    VIDEOS_DIR.mkdir(parents=True, exist_ok=True)
    timestamp = int(time.time())
    clean_prefix = "".join([c if c.isalnum() else "_" for c in song.get("id", filename_prefix)])[:30]
    out_filename = f"{clean_prefix}_{timestamp}.mp4"
    out_path = VIDEOS_DIR / out_filename

    # Determine audio source & duration
    audio_path = song.get("audio_path")
    duration = float(song.get("duration", 0.0))

    if audio_path and Path(audio_path).exists():
        # Get actual audio duration via ffprobe if possible
        try:
            probe_cmd = [
                "ffprobe", "-v", "error", "-show_entries", "format=duration",
                "-of", "default=noprint_wrappers=1:nokey=1", audio_path
            ]
            probe_res = subprocess.run(probe_cmd, capture_output=True, text=True)
            if probe_res.returncode == 0 and probe_res.stdout.strip():
                duration = float(probe_res.stdout.strip())
        except Exception:
            pass

    if duration <= 0:
        verses = song.get("verses", [])
        if verses:
            duration = max([v.get("end_time") or (v.get("start_time") or 0.0) + 5.0 for v in verses]) + 3.0
        else:
            duration = 15.0

    # Create temporary ASS subtitle file
    ass_content = generate_ass_subtitles(song, duration)
    ass_path = VIDEOS_DIR / f"temp_{timestamp}.ass"
    with open(ass_path, "w", encoding="utf-8") as f:
        f.write(ass_content)

    try:
        # Build FFmpeg command
        if audio_path and Path(audio_path).exists():
            cmd = [
                "ffmpeg", "-y",
                "-f", "lavfi", "-i", f"color=c=#0B1120:s=1280x720:d={duration}",
                "-i", audio_path,
                "-vf", f"ass={escape_filter_path(ass_path)}",
                "-c:v", "libx264", "-preset", "fast", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-b:a", "192k",
                "-shortest",
                str(out_path)
            ]
        else:
            # Fallback to silent stereo audio track
            cmd = [
                "ffmpeg", "-y",
                "-f", "lavfi", "-i", f"color=c=#0B1120:s=1280x720:d={duration}",
                "-f", "lavfi", "-i", f"anullsrc=r=44100:cl=stereo",
                "-vf", f"ass={escape_filter_path(ass_path)}",
                "-c:v", "libx264", "-preset", "fast", "-pix_fmt", "yuv420p",
                "-c:a", "aac",
                "-t", str(duration),
                str(out_path)
            ]

        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            raise RuntimeError(f"FFmpeg failed (code {res.returncode}): {res.stderr}")

    finally:
        # Clean up temporary ASS file
        if ass_path.exists():
            ass_path.unlink()

    return str(out_path)

def generate_performance_video(songs: List[Dict[str, Any]], title: str = "הופעה", filename_prefix: str = "performance") -> str:
    """Render a unified full-length MP4 movie containing all songs in the performance."""
    VIDEOS_DIR.mkdir(parents=True, exist_ok=True)
    if not songs:
        raise ValueError("No songs provided for performance video")

    # If only 1 song, simply generate that song's video
    if len(songs) == 1:
        return generate_karaoke_video(songs[0], filename_prefix=filename_prefix)

    # Generate video for each song
    song_video_paths = []
    for idx, song in enumerate(songs, 1):
        prefix = f"{filename_prefix}_part{idx}"
        v_path = generate_karaoke_video(song, filename_prefix=prefix)
        song_video_paths.append(v_path)

    timestamp = int(time.time())
    clean_prefix = "".join([c if c.isalnum() else "_" for c in filename_prefix])[:30]
    out_filename = f"{clean_prefix}_{timestamp}.mp4"
    out_path = VIDEOS_DIR / out_filename

    # Concat file list
    concat_list_file = VIDEOS_DIR / f"concat_{timestamp}.txt"
    with open(concat_list_file, "w", encoding="utf-8") as f:
        for vp in song_video_paths:
            f.write(f"file '{vp}'\n")

    try:
        cmd = [
            "ffmpeg", "-y",
            "-f", "concat", "-safe", "0",
            "-i", str(concat_list_file),
            "-c", "copy",
            str(out_path)
        ]
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            raise RuntimeError(f"FFmpeg performance video concat failed: {res.stderr}")
    finally:
        if concat_list_file.exists():
            concat_list_file.unlink()

    return str(out_path)

