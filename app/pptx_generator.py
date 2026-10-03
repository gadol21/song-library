import io
import subprocess
import tempfile
import time
from pathlib import Path
from typing import List, Dict, Any, Optional

from PIL import Image
from pptx import Presentation
from pptx.util import Inches, Pt
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.enum.shapes import MSO_SHAPE
from pptx.opc.constants import RELATIONSHIP_TYPE as RT
from pptx.opc.package import Part
from pptx.opc.packuri import PackURI
from pptx.oxml import parse_xml

from app.database import PRESENTATIONS_DIR, detect_language
from app.text_fit import fit_font_size, line_height_em
from app.youtube import ffprobe_duration
from app.theme import resolve_theme, hex_to_rgb

# Text box geometry (inches). python-pptx text boxes have 0.1in side and 0.05in top/bottom insets.
INSET_X, INSET_Y = 0.1, 0.05
TITLE_BOX = (1.5, 2.3, 10.333, 3.5)     # left, top, width, height
HEADER_BOX = (1.0, 0.5, 11.333, 0.8)
VERSE_BOX = (1.0, 1.6, 11.333, 5.2)
VERSE_LINE_SPACING = 1.25
# Font size limits (pt)
MAX_TITLE_PT, MAX_HEADER_PT, MAX_VERSE_PT, MIN_VERSE_PT = 96, 40, 110, 20
# Vertical room the song title may use on its slide (the rest is for the show name and artist)
TITLE_HEIGHT_IN = 2.0

# Audio embedding. PowerPoint plays only these formats; anything else is converted to MP3 first.
AUDIO_CONTENT_TYPES = {".mp3": "audio/mpeg", ".m4a": "audio/mp4", ".wav": "audio/wav"}
MEDIA_RELATIONSHIP = "http://schemas.microsoft.com/office/2007/relationships/media"
# Seconds the title slide is shown when the first verse starts at (almost) 0, so the audio starts on the verse slide
LEAD_IN_SECONDS = 3.0
MIN_TITLE_SECONDS = 2.0
LAST_VERSE_FALLBACK_SECONDS = 6.0
NS = ('xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
      'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
      'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"')

def _rgb(hex_color: str) -> RGBColor:
    return RGBColor(*hex_to_rgb(hex_color))

def _verse_lines(verse: Dict[str, Any]) -> List[str]:
    return [l.strip() for l in verse.get("text", "").strip().split("\n") if l.strip()]

def _fit_pt(blocks: List[List[str]], box_w_in: float, box_h_in: float, line_spacing: float, max_pt: float) -> float:
    """Largest point size at which every block fits inside a text box of the given size (inches)."""
    return fit_font_size(
        blocks,
        box_width=(box_w_in - 2 * INSET_X) * 72,
        box_height=(box_h_in - 2 * INSET_Y) * 72,
        line_height=line_height_em() * line_spacing,
        max_size=max_pt,
    )

def _header_text(song: Dict[str, Any], verse: Dict[str, Any], verse_index: int, total_verses: int) -> str:
    song_title = song.get("title", "")
    if detect_language(verse.get("text", "") + song_title) == "he":
        progress_str = f"בית {verse_index} מתוך {total_verses}"
    else:
        progress_str = f"Verse {verse_index} of {total_verses}"
    return f"{song_title}  •  {progress_str}"

def song_font_sizes(song: Dict[str, Any]) -> Dict[str, float]:
    """Point sizes used for every slide of a song: the largest that fit its longest title/header/verse."""
    verses = song.get("verses", [])
    headers = [[_header_text(song, v, i, len(verses))] for i, v in enumerate(verses, 1)]
    title = song.get("title", "שיר ללא שם")
    return {
        "title": _fit_pt([[title]], TITLE_BOX[2], TITLE_HEIGHT_IN, 1.0, MAX_TITLE_PT),
        "header": _fit_pt(headers, HEADER_BOX[2], HEADER_BOX[3], 1.0, MAX_HEADER_PT),
        "verse": max(MIN_VERSE_PT, _fit_pt([_verse_lines(v) for v in verses],
                                           VERSE_BOX[2], VERSE_BOX[3], VERSE_LINE_SPACING, MAX_VERSE_PT)),
    }

def _set_slide_background(slide, prs_width, prs_height, theme: Dict[str, str]):
    """Draw a background rectangle covering the full slide."""
    bg = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, 0, 0, prs_width, prs_height)
    bg.fill.solid()
    bg.fill.fore_color.rgb = _rgb(theme["bg_color"])
    bg.line.fill.background() # no border
    return bg

def _apply_rtl_if_needed(paragraph, text: str):
    """Enable native RTL paragraph formatting if text contains Hebrew."""
    lang = detect_language(text)
    if lang == "he":
        paragraph.alignment = PP_ALIGN.RIGHT
        try:
            paragraph._p.get_or_add_pPr().set('rtl', '1')
        except Exception:
            pass
    else:
        paragraph.alignment = PP_ALIGN.CENTER
    return lang

def create_song_title_slide(prs, song: Dict[str, Any], theme: Dict[str, str], sizes: Dict[str, float],
                            perf_title: Optional[str] = None):
    """Add a stylish title slide introducing the song."""
    slide = prs.slides.add_slide(prs.slide_layouts[6])
    _set_slide_background(slide, prs.slide_width, prs.slide_height, theme)

    # Accent decorative bar on top
    accent_bar = slide.shapes.add_shape(
        MSO_SHAPE.RECTANGLE,
        Inches(1.5), Inches(1.8), Inches(10.333), Inches(0.08)
    )
    accent_bar.fill.solid()
    accent_bar.fill.fore_color.rgb = _rgb(theme["title_color"])
    accent_bar.line.fill.background()

    # Title & Artist Textbox
    tb = slide.shapes.add_textbox(*(Inches(v) for v in TITLE_BOX))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = MSO_ANCHOR.MIDDLE

    # Optional Performance header
    if perf_title:
        p_perf = tf.paragraphs[0]
        p_perf.text = perf_title
        p_perf.font.name = "Arial"
        p_perf.font.size = Pt(20)
        p_perf.font.color.rgb = _rgb(theme["text_color"])
        _apply_rtl_if_needed(p_perf, perf_title)
        p_title = tf.add_paragraph()
    else:
        p_title = tf.paragraphs[0]

    # Song Title
    song_title = song.get("title", "שיר ללא שם")
    p_title.text = song_title
    p_title.font.name = "Arial"
    p_title.font.size = Pt(sizes["title"])
    p_title.font.bold = True
    p_title.font.color.rgb = _rgb(theme["title_color"])
    p_title.space_before = Pt(14)
    _apply_rtl_if_needed(p_title, song_title)

    # Artist
    artist = song.get("artist", "")
    if artist:
        p_artist = tf.add_paragraph()
        p_artist.text = artist
        p_artist.font.name = "Arial"
        p_artist.font.size = Pt(32)
        p_artist.font.color.rgb = _rgb(theme["text_color"])
        p_artist.space_before = Pt(10)
        _apply_rtl_if_needed(p_artist, artist)

    return slide

def create_verse_slide(prs, song: Dict[str, Any], verse: Dict[str, Any], verse_index: int, total_verses: int,
                       theme: Dict[str, str], sizes: Dict[str, float]):
    """Add a high-readability verse slide with song header and progress indicator."""
    slide = prs.slides.add_slide(prs.slide_layouts[6])
    _set_slide_background(slide, prs.slide_width, prs.slide_height, theme)

    # Top Header: Song Title & Progress
    header_tb = slide.shapes.add_textbox(*(Inches(v) for v in HEADER_BOX))
    htf = header_tb.text_frame
    htf.word_wrap = True
    htf.vertical_anchor = MSO_ANCHOR.MIDDLE
    hp = htf.paragraphs[0]

    header_text = _header_text(song, verse, verse_index, total_verses)
    hp.text = header_text
    hp.font.name = "Arial"
    hp.font.size = Pt(sizes["header"])
    hp.font.bold = True
    hp.font.color.rgb = _rgb(theme["title_color"])
    _apply_rtl_if_needed(hp, header_text)

    # Subtle divider line under header
    divider = slide.shapes.add_shape(
        MSO_SHAPE.RECTANGLE,
        Inches(1.0), Inches(1.3), Inches(11.333), Inches(0.02)
    )
    divider.fill.solid()
    divider.fill.fore_color.rgb = _rgb(theme["title_color"])
    divider.line.fill.background()

    # Verse Text Box (Centered and Large)
    verse_tb = slide.shapes.add_textbox(*(Inches(v) for v in VERSE_BOX))
    vtf = verse_tb.text_frame
    vtf.word_wrap = True
    vtf.vertical_anchor = MSO_ANCHOR.MIDDLE

    # Same size on every verse slide of the song (fitted to its longest verse)
    for i, line in enumerate(_verse_lines(verse)):
        p = vtf.paragraphs[0] if i == 0 else vtf.add_paragraph()
        p.text = line
        p.font.name = "Arial"
        p.font.size = Pt(sizes["verse"])
        p.font.bold = True
        p.font.color.rgb = _rgb(theme["text_color"])
        p.line_spacing = VERSE_LINE_SPACING
        _apply_rtl_if_needed(p, line)

    return slide

def _audio_duration(song: Dict[str, Any], audio_path: str) -> float:
    """Length of the song's audio in seconds (ffprobe), falling back to the stored duration, or 0 if unknown."""
    return ffprobe_duration(Path(audio_path)) or float(song.get("duration") or 0.0)

def _powerpoint_audio(audio_path: str, tmp_dir: str) -> Optional[Path]:
    """The audio file itself if PowerPoint can play it, otherwise an MP3 copy in tmp_dir (None if conversion fails)."""
    path = Path(audio_path)
    if path.suffix.lower() in AUDIO_CONTENT_TYPES:
        return path
    out = Path(tmp_dir) / (path.stem + ".mp3")
    try:
        res = subprocess.run(["ffmpeg", "-y", "-i", str(path), "-vn", "-c:a", "libmp3lame", "-b:a", "192k", str(out)],
                             capture_output=True, text=True, encoding="utf-8", errors="replace")
        return out if res.returncode == 0 and out.exists() else None
    except Exception:
        return None

def _slide_durations(song: Dict[str, Any], audio_length: float) -> Optional[Dict[str, Any]]:
    """Seconds each of the song's slides is shown so the deck follows the audio, or None if the verses have no timings.

    Returns {"title": seconds, "verses": [seconds, ...], "audio_on_title": bool}. A verse slide lasts from its start
    to the next verse's start (so gaps stay on screen and the slides never drift from the audio); the last one lasts
    until its end_time, else the end of the audio. The audio starts on the title slide, which then lasts until the first
    verse starts; if that is (almost) at 0 the title slide is a short lead-in and the audio starts on the first verse slide.
    """
    verses = song.get("verses", [])
    starts = [v.get("start_time") for v in verses]
    if not verses or any(t is None for t in starts) or any(b < a for a, b in zip(starts, starts[1:])):
        return None
    starts = [float(t) for t in starts]
    last_end = verses[-1].get("end_time")
    if last_end is not None and float(last_end) > starts[-1]:
        end = float(last_end)
    elif audio_length > starts[-1]:
        end = audio_length
    else:
        end = starts[-1] + LAST_VERSE_FALLBACK_SECONDS
    durations = [b - a for a, b in zip(starts, starts[1:] + [end])]
    if starts[0] >= MIN_TITLE_SECONDS:
        return {"title": starts[0], "verses": durations, "audio_on_title": True}
    # The first verse slide carries the audio, which starts at 0 of the song, so it also covers the time before its start
    durations[0] += starts[0]
    return {"title": LEAD_IN_SECONDS, "verses": durations, "audio_on_title": False}

def _set_auto_advance(slide, seconds: float):
    """Make the slide advance by itself after the given time in slide show mode."""
    slide._element.append(parse_xml(f'<p:transition {NS} advTm="{max(1, round(seconds * 1000))}"/>'))

def _icon_png() -> bytes:
    """A tiny transparent picture: the face of the audio object (PowerPoint hides it while the show runs)."""
    buf = io.BytesIO()
    Image.new("RGBA", (8, 8), (0, 0, 0, 0)).save(buf, "PNG")
    return buf.getvalue()

def _embed_audio(slide, audio_file: Path, across_slides: int):
    """Embed the audio in the slide, set to start automatically and keep playing across the next slides."""
    package = slide.part.package
    ext = audio_file.suffix.lower()
    partname = package.next_partname("/ppt/media/audio%d" + ext)
    audio_part = Part(PackURI(partname), AUDIO_CONTENT_TYPES[ext], package, audio_file.read_bytes())
    audio_rid = slide.part.relate_to(audio_part, RT.AUDIO)
    media_rid = slide.part.relate_to(audio_part, MEDIA_RELATIONSHIP)

    pic = slide.shapes.add_picture(io.BytesIO(_icon_png()), Inches(0.1), Inches(0.1), Inches(0.3), Inches(0.3))
    pic.name = "Song audio"
    shape_id = pic.shape_id
    pic._element.nvPicPr.cNvPr.append(parse_xml(f'<a:hlinkClick {NS} r:id="" action="ppaction://media"/>'))
    nv_pr = pic._element.nvPicPr.nvPr
    nv_pr.append(parse_xml(f'<a:audioFile {NS} r:link="{audio_rid}"/>'))
    nv_pr.append(parse_xml(
        f'<p:extLst {NS}><p:ext uri="{{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}}">'
        f'<p14:media xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" r:embed="{media_rid}"/>'
        f'</p:ext></p:extLst>'))

    # Timing tree: play from the start as soon as the slide appears; the media node spans `across_slides` slides
    slide._element.append(parse_xml(f"""
<p:timing {NS}><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>
 <p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>
  <p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"><p:tn val="2"/></p:cond></p:stCondLst><p:childTnLst>
   <p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
    <p:par><p:cTn id="5" presetID="1" presetClass="mediacall" presetSubtype="0" fill="hold" nodeType="afterEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
     <p:cmd type="call" cmd="playFrom(0.0)"><p:cBhvr><p:cTn id="6" dur="1" fill="hold"/><p:tgtEl><p:spTgt spid="{shape_id}"/></p:tgtEl></p:cBhvr></p:cmd>
    </p:childTnLst></p:cTn></p:par>
   </p:childTnLst></p:cTn></p:par>
  </p:childTnLst></p:cTn></p:par>
 </p:childTnLst></p:cTn>
 <p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst>
 <p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>
 <p:audio><p:cMediaNode vol="100000" numSld="{across_slides}" showWhenStopped="0"><p:cTn id="7" fill="hold" display="0">
  <p:stCondLst><p:cond delay="indefinite"/></p:stCondLst>
  <p:endCondLst><p:cond evt="onStopAudio" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:endCondLst></p:cTn>
  <p:tgtEl><p:spTgt spid="{shape_id}"/></p:tgtEl></p:cMediaNode></p:audio>
</p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"""))

def generate_presentation(songs: List[Dict[str, Any]], title: str = "שירה בציבור", filename_prefix: str = "performance",
                          theme: Optional[Dict[str, Any]] = None) -> str:
    """Generate a complete 16:9 widescreen presentation deck for a setlist or song.

    A song with audio gets it embedded; if its verses are timed, the slides also advance by themselves,
    each shown for exactly its verse, and the audio starts playing on its own.
    """
    theme = resolve_theme(theme)
    PRESENTATIONS_DIR.mkdir(parents=True, exist_ok=True)
    prs = Presentation()
    prs.slide_width = Inches(13.333)  # 16:9 Widescreen
    prs.slide_height = Inches(7.5)

    with tempfile.TemporaryDirectory(prefix="singalong-pptx-") as tmp:
        for song in songs:
            verses = song.get("verses", [])
            sizes = song_font_sizes(song)
            # Song Title Slide
            slides = [create_song_title_slide(prs, song, theme, sizes, perf_title=title)]

            # Verse Slides
            total_verses = len(verses)
            for idx, verse in enumerate(verses, 1):
                slides.append(create_verse_slide(prs, song, verse, idx, total_verses, theme, sizes))

            audio_path = song.get("audio_path")
            if not audio_path or not Path(audio_path).exists():
                continue
            audio_file = _powerpoint_audio(audio_path, tmp)
            if audio_file is None:
                continue
            plan = _slide_durations(song, _audio_duration(song, str(audio_path)))
            audio_slide = 1 if plan and not plan["audio_on_title"] and len(slides) > 1 else 0
            if plan:
                for slide, seconds in zip(slides, [plan["title"], *plan["verses"]]):
                    _set_auto_advance(slide, seconds)
            _embed_audio(slides[audio_slide], audio_file, across_slides=len(slides) - audio_slide)

        # Save inside the temp dir's lifetime: a converted audio copy is read from it
        timestamp = int(time.time())
        clean_prefix = "".join([c if c.isalnum() else "_" for c in filename_prefix])[:30]
        out_filename = f"{clean_prefix}_{timestamp}.pptx"
        out_path = PRESENTATIONS_DIR / out_filename
        prs.save(str(out_path))

    return str(out_path)
