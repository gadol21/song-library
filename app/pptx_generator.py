import os
import time
from pathlib import Path
from typing import List, Dict, Any, Optional

from pptx import Presentation
from pptx.util import Inches, Pt
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.enum.shapes import MSO_SHAPE

from app.database import PRESENTATIONS_DIR, detect_language
from app.text_fit import fit_font_size, line_height_em
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

def generate_presentation(songs: List[Dict[str, Any]], title: str = "שירה בציבור", filename_prefix: str = "performance",
                          theme: Optional[Dict[str, Any]] = None) -> str:
    """Generate a complete 16:9 widescreen presentation deck for a setlist or song."""
    theme = resolve_theme(theme)
    PRESENTATIONS_DIR.mkdir(parents=True, exist_ok=True)
    prs = Presentation()
    prs.slide_width = Inches(13.333)  # 16:9 Widescreen
    prs.slide_height = Inches(7.5)

    for song in songs:
        verses = song.get("verses", [])
        sizes = song_font_sizes(song)
        # Song Title Slide
        create_song_title_slide(prs, song, theme, sizes, perf_title=title)

        # Verse Slides
        total_verses = len(verses)
        for idx, verse in enumerate(verses, 1):
            create_verse_slide(prs, song, verse, idx, total_verses, theme, sizes)

    # Save output file
    timestamp = int(time.time())
    clean_prefix = "".join([c if c.isalnum() else "_" for c in filename_prefix])[:30]
    out_filename = f"{clean_prefix}_{timestamp}.pptx"
    out_path = PRESENTATIONS_DIR / out_filename
    prs.save(str(out_path))

    return str(out_path)
