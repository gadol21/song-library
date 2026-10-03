import math
import os
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Dict, Any, List, Optional, Tuple

from app.database import VIDEOS_DIR, detect_language
from app.text_fit import fit_font_size, line_height_em
from app.theme import resolve_theme, hex_to_rgb

VIDEO_W, VIDEO_H = 1280, 720          # the canvas the layout is designed on (also the subtitle script size)
OUTPUT_W, OUTPUT_H = 1920, 1080       # size of the rendered video; the subtitle renderer scales the canvas up to it
# Screen regions (px): title strip at top, next-verse preview at bottom, verse fills the middle
HEADER_MARGIN_X, HEADER_MARGIN_TOP, HEADER_HEIGHT = 40, 30, 90
VERSE_MARGIN_X, VERSE_HEIGHT = 70, 460
NEXT_FONT_SIZE, NEXT_MARGIN_BOTTOM = 30, 40
MAX_HEADER_SIZE, MAX_VERSE_SIZE, MIN_VERSE_SIZE = 72, 160, 24
END_PADDING = 0.1  # seconds the output may run past the song's length (see generate_karaoke_video)
MIN_FRAME_RATE, MAX_FRAME_RATE = 1, 30  # frames per second allowed for a constant-frame-rate video
DEFAULT_FRAME_RATE = 5  # used when nothing is specified: a constant rate plays in every player

def check_frame_rate(frame_rate: Optional[float]) -> Optional[float]:
    """None means a variable frame rate; otherwise a constant rate in frames per second within the allowed range."""
    if frame_rate is None:
        return None
    if isinstance(frame_rate, bool):
        raise ValueError("קצב הפריימים חייב להיות מספר")
    try:
        rate = float(frame_rate)
    except (TypeError, ValueError):
        raise ValueError("קצב הפריימים חייב להיות מספר")
    if not (MIN_FRAME_RATE <= rate <= MAX_FRAME_RATE):  # also rejects NaN
        raise ValueError(f"קצב הפריימים חייב להיות בין {MIN_FRAME_RATE} ל-{MAX_FRAME_RATE} פריימים לשנייה")
    return rate

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
        f"PlayResX: {VIDEO_W}",
        f"PlayResY: {VIDEO_H}",
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

def parse_ass_time(stamp: str) -> float:
    """Seconds from an ASS timestamp (H:MM:SS.cs)."""
    hours, minutes, seconds = stamp.split(":")
    return int(hours) * 3600 + int(minutes) * 60 + float(seconds)

def plan_screens(ass_content: str) -> Tuple[List[float], str]:
    """Find the moments the picture changes, and retime the subtitle script so each screen can be drawn once.

    The picture only changes when a subtitle appears or disappears, so a song has a few hundred distinct
    screens rather than thousands of frames. Screen i is shown from times[i] to times[i + 1]; in the
    returned script it is on screen during second i instead.
    """
    lines = ass_content.split("\n")
    events = [line.split(",", 9) for line in lines if line.startswith("Dialogue:")]
    times = sorted({parse_ass_time(e[1]) for e in events} | {parse_ass_time(e[2]) for e in events})
    slot = {t: i for i, t in enumerate(times)}

    def retime(line: str) -> str:
        fields = line.split(",", 9)  # the text is the last field and may contain commas
        fields[1] = format_ass_time(slot[parse_ass_time(fields[1])])
        fields[2] = format_ass_time(slot[parse_ass_time(fields[2])])
        return ",".join(fields)

    return times, "\n".join(retime(line) if line.startswith("Dialogue:") else line for line in lines)

def run_ffmpeg(cmd: List[str], what: str) -> None:
    res = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if res.returncode != 0:
        raise RuntimeError(f"FFmpeg failed while {what} (code {res.returncode}): {res.stderr}")

def render_screens(ass_content: str, bg_color: str, work_dir: Path, frame_rate: Optional[float] = None) -> Path:
    """Draw every distinct screen once, as a PNG, and return an FFmpeg concat list that shows each for the right time.

    With a frame_rate, each change is moved to the first frame at or after it, so every screen lasts a whole
    number of frames. (FFmpeg counts the time of an image sequence in 1/25 s ticks; exact multiples of the frame
    length keep that rounding from ever tipping a change into the wrong frame.)
    """
    times, retimed = plan_screens(ass_content)
    count = len(times) - 1
    if frame_rate is None:
        shown_from = times
    else:
        shown_from = [math.ceil(round(t * frame_rate, 6)) / frame_rate for t in times]
    script = work_dir / "screens.ass"
    script.write_text(retimed, encoding="utf-8")
    # One frame per second, taken in the middle of that second, i.e. in the middle of screen i
    run_ffmpeg([
        "ffmpeg", "-y",
        "-f", "lavfi", "-i", f"color=c={bg_color}:s={OUTPUT_W}x{OUTPUT_H}:r=1:d={count}",
        "-vf", f"setpts=PTS+0.5/TB,ass={escape_filter_path(script)}",
        "-fps_mode", "passthrough", "-c:v", "png", "-compression_level", "1",
        str(work_dir / "screen_%05d.png"),
    ], "drawing the screens")

    lines = []
    for i in range(count):
        length = shown_from[i + 1] - shown_from[i]
        if length < 1e-9:  # shorter than one frame: no frame would show this screen
            continue
        lines.append(f"file 'screen_{i + 1:05d}.png'")
        lines.append(f"duration {length:.6f}")
    lines.append(f"file 'screen_{count:05d}.png'")  # the concat demuxer needs the last file repeated to honor its duration
    concat_list = work_dir / "screens.txt"
    concat_list.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return concat_list

def generate_karaoke_video(song: Dict[str, Any], filename_prefix: str = "karaoke", theme: Optional[Dict[str, Any]] = None,
                           frame_rate: Optional[float] = DEFAULT_FRAME_RATE) -> str:
    """Render a 1080p Full HD MP4 karaoke video for a song using FFmpeg.

    The picture only changes when the subtitles change, so each distinct screen is drawn only once.
    A number (the default, DEFAULT_FRAME_RATE) gives a constant-frame-rate video at that many frames per second,
    which plays everywhere. frame_rate=None, passed explicitly, gives a variable-frame-rate video with one frame
    per screen, held for exactly as long as it is shown: the fastest to make, but not every player handles it
    (VLC does, Windows Media Player does not).
    """
    frame_rate = check_frame_rate(frame_rate)
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
            probe_res = subprocess.run(probe_cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
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

    ass_content = generate_ass_subtitles(song, duration, theme)

    with tempfile.TemporaryDirectory(prefix="singalong-video-") as tmp:
        screens = render_screens(ass_content, theme["bg_color"], Path(tmp), frame_rate)

        if frame_rate is None:
            # -g 30: a keyframe at least every 30 screens, so seeking in a player stays quick.
            # -bf 0: still pictures gain nothing from B-frames, and without them the length stored for each picture is exact.
            video_args = ["-fps_mode:v", "vfr", "-c:v", "libx264", "-preset", "fast", "-pix_fmt", "yuv420p", "-g", "30", "-bf", "0"]
            # Stop a moment after the end of the song, not exactly at it: the final screen's length is only
            # recorded correctly if a frame follows it, so the video track then ends with the audio.
            stop_at = ["-t", f"{duration + END_PADDING:.2f}"]
        else:
            video_args = ["-vf", f"fps={frame_rate:g}", "-c:v", "libx264", "-preset", "fast", "-pix_fmt", "yuv420p"]
            stop_at = ["-t", f"{duration:.2f}"]
        if audio_path and Path(audio_path).exists():
            audio_input = ["-i", audio_path]
            audio_output = ["-c:a", "aac", "-b:a", "192k", *stop_at]
        else:
            # Fallback to a silent stereo audio track
            audio_input = ["-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo"]
            audio_output = ["-c:a", "aac", *stop_at]

        run_ffmpeg([
            "ffmpeg", "-y",
            "-f", "concat", "-i", str(screens),
            *audio_input,
            *video_args,
            *audio_output,
            str(out_path),
        ], "encoding the video")

    return str(out_path)

def generate_performance_video(songs: List[Dict[str, Any]], title: str = "הופעה", filename_prefix: str = "performance", theme: Optional[Dict[str, Any]] = None,
                               frame_rate: Optional[float] = DEFAULT_FRAME_RATE) -> str:
    """Render a unified full-length MP4 movie containing all songs in the performance (see generate_karaoke_video for frame_rate)."""
    frame_rate = check_frame_rate(frame_rate)
    VIDEOS_DIR.mkdir(parents=True, exist_ok=True)
    if not songs:
        raise ValueError("No songs provided for performance video")

    # If only 1 song, simply generate that song's video
    if len(songs) == 1:
        return generate_karaoke_video(songs[0], filename_prefix=filename_prefix, theme=theme, frame_rate=frame_rate)

    # Generate video for each song
    song_video_paths = []
    for idx, song in enumerate(songs, 1):
        prefix = f"{filename_prefix}_part{idx}"
        v_path = generate_karaoke_video(song, filename_prefix=prefix, theme=theme, frame_rate=frame_rate)
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
        res = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if res.returncode != 0:
            raise RuntimeError(f"FFmpeg performance video concat failed: {res.stderr}")
    finally:
        if concat_list_file.exists():
            concat_list_file.unlink()

    return str(out_path)

