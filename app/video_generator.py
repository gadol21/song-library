import os
import subprocess
import time
from pathlib import Path
from typing import Dict, Any, List, Optional

from app.database import VIDEOS_DIR, detect_language
from app.text_fit import fit_font_size, line_height_em
from app.theme import resolve_theme, hex_to_rgb

VIDEO_W, VIDEO_H = 1280, 720
# Screen regions (px): title strip at top, next-verse preview at bottom, verse fills the middle
HEADER_MARGIN_X, HEADER_MARGIN_TOP, HEADER_HEIGHT = 40, 30, 90
VERSE_MARGIN_X, VERSE_HEIGHT = 70, 460
NEXT_FONT_SIZE, NEXT_MARGIN_BOTTOM = 30, 40
MAX_HEADER_SIZE, MAX_VERSE_SIZE, MIN_VERSE_SIZE = 72, 160, 24

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

def ass_color(hex_color: str, alpha: int = 0) -> str:
    """Convert #RRGGBB to ASS &HAABBGGRR (alpha 0 = opaque, 255 = transparent)."""
    r, g, b = hex_to_rgb(hex_color)
    return f"&H{alpha:02X}{b:02X}{g:02X}{r:02X}"

def verse_lines(verse: Dict[str, Any]) -> List[str]:
    """Non-empty, stripped lines of a verse's text."""
    return [line.strip() for line in verse.get("text", "").strip().split("\n") if line.strip()]

def fit_ass_font_size(blocks: List[List[str]], box_width: float, box_height: float, max_size: float) -> int:
    """Largest ASS Fontsize at which every block of lines fits the box.

    libass treats Fontsize as the font's line height (ascent + descent), not its em size,
    so fit in em units and convert.
    """
    lh = line_height_em()
    em = fit_font_size(blocks, box_width, box_height, line_height=lh, max_size=max_size / lh)
    return int(em * lh)

def generate_ass_subtitles(song: Dict[str, Any], total_duration: float, theme: Optional[Dict[str, Any]] = None) -> str:
    """Generate Advanced SubStation Alpha (.ass) subtitle file for karaoke rendering."""
    theme = resolve_theme(theme)
    title = song.get("title", "")
    artist = song.get("artist", "")
    verses = song.get("verses", [])
    
    # Sort verses by start time
    sorted_verses = sorted(verses, key=lambda v: v.get("start_time") or 0.0)

    header_text = f"{title} - {artist}" if artist else title

    # One font size for the whole song: the largest at which its longest verse still fits
    header_size = fit_ass_font_size([[header_text]], VIDEO_W - 2 * HEADER_MARGIN_X, HEADER_HEIGHT, MAX_HEADER_SIZE)
    verse_size = fit_ass_font_size([verse_lines(v) for v in sorted_verses],
                                   VIDEO_W - 2 * VERSE_MARGIN_X, VERSE_HEIGHT, MAX_VERSE_SIZE)
    verse_size = max(MIN_VERSE_SIZE, verse_size)

    title_col = ass_color(theme["title_color"])
    text_col = ass_color(theme["text_color"])
    # Next-verse preview: text color, partly transparent so it reads as secondary
    next_col = ass_color(theme["text_color"], alpha=0x70)

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
        f"Style: Header,Arial,{header_size},{title_col},&H00000000,&H00000000,&H80000000,1,0,0,0,100,100,0,0,1,2,1,8,{HEADER_MARGIN_X},{HEADER_MARGIN_X},{HEADER_MARGIN_TOP},1",
        # Active verse: centered, as large as fits, with a dark outline
        f"Style: ActiveVerse,Arial,{verse_size},{text_col},&H00000000,&H00000000,&HA0000000,1,0,0,0,100,100,0,0,1,3,2,5,{VERSE_MARGIN_X},{VERSE_MARGIN_X},0,1",
        # Next verse preview: bottom, smaller, faded text color
        f"Style: NextVerse,Arial,{NEXT_FONT_SIZE},{next_col},&H00000000,&H00000000,&HA0000000,0,0,0,0,100,100,0,0,1,2,1,2,{VERSE_MARGIN_X},{VERSE_MARGIN_X},{NEXT_MARGIN_BOTTOM},1",
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
        formatted_text = r"\N".join(verse_lines(v))

        start_str = format_ass_time(start)
        end_str = format_ass_time(end)

        # Active verse
        ass_lines.append(f"Dialogue: 1,{start_str},{end_str},ActiveVerse,,0,0,0,,{formatted_text}")

        # Next verse preview (if there is an upcoming verse)
        if idx + 1 < len(sorted_verses):
            next_lines = verse_lines(sorted_verses[idx + 1])
            first_line = next_lines[0] if next_lines else ""
            lang = detect_language(first_line)
            label = "הבא: " if lang == "he" else "Next: "
            ass_lines.append(f"Dialogue: 0,{start_str},{end_str},NextVerse,,0,0,0,,{label}{first_line}")

    return "\n".join(ass_lines)

def generate_karaoke_video(song: Dict[str, Any], filename_prefix: str = "karaoke", theme: Optional[Dict[str, Any]] = None) -> str:
    """Render a 720p HD MP4 karaoke video for a song using FFmpeg."""
    theme = resolve_theme(theme)
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
    ass_content = generate_ass_subtitles(song, duration, theme)
    ass_path = VIDEOS_DIR / f"temp_{timestamp}.ass"
    with open(ass_path, "w", encoding="utf-8") as f:
        f.write(ass_content)

    try:
        # Build FFmpeg command
        if audio_path and Path(audio_path).exists():
            cmd = [
                "ffmpeg", "-y",
                "-f", "lavfi", "-i", f"color=c={theme['bg_color']}:s={VIDEO_W}x{VIDEO_H}:r=5:d={duration}",
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
                "-f", "lavfi", "-i", f"color=c={theme['bg_color']}:s={VIDEO_W}x{VIDEO_H}:r=5:d={duration}",
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

def generate_performance_video(songs: List[Dict[str, Any]], title: str = "הופעה", filename_prefix: str = "performance", theme: Optional[Dict[str, Any]] = None) -> str:
    """Render a unified full-length MP4 movie containing all songs in the performance."""
    VIDEOS_DIR.mkdir(parents=True, exist_ok=True)
    if not songs:
        raise ValueError("No songs provided for performance video")

    # If only 1 song, simply generate that song's video
    if len(songs) == 1:
        return generate_karaoke_video(songs[0], filename_prefix=filename_prefix, theme=theme)

    # Generate video for each song
    song_video_paths = []
    for idx, song in enumerate(songs, 1):
        prefix = f"{filename_prefix}_part{idx}"
        v_path = generate_karaoke_video(song, filename_prefix=prefix, theme=theme)
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

