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

# Theme Colors
BG_COLOR = RGBColor(15, 23, 42)          # Deep Midnight Slate (#0F172A)
CARD_BG = RGBColor(30, 41, 59)           # Lighter Slate (#1E293B)
ACCENT_GOLD = RGBColor(245, 158, 11)     # Warm Stage Amber (#F59E0B)
ACCENT_CYAN = RGBColor(56, 189, 248)     # Vibrant Sky (#38BDF8)
TEXT_WHITE = RGBColor(255, 255, 255)     # High Contrast White
TEXT_MUTED = RGBColor(148, 163, 184)     # Slate Light Muted (#94A3B8)
BORDER_COLOR = RGBColor(51, 65, 85)      # Slate Border (#334155)

def _set_slide_background(slide, prs_width, prs_height):
    """Draw a dark background rectangle covering the full slide."""
    bg = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, 0, 0, prs_width, prs_height)
    bg.fill.solid()
    bg.fill.fore_color.rgb = BG_COLOR
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

def create_song_title_slide(prs, song: Dict[str, Any], perf_title: Optional[str] = None):
    """Add a stylish title slide introducing the song."""
    slide = prs.slides.add_slide(prs.slide_layouts[6])
    _set_slide_background(slide, prs.slide_width, prs.slide_height)

    # Accent decorative bar on top
    accent_bar = slide.shapes.add_shape(
        MSO_SHAPE.RECTANGLE,
        Inches(1.5), Inches(1.8), Inches(10.333), Inches(0.08)
    )
    accent_bar.fill.solid()
    accent_bar.fill.fore_color.rgb = ACCENT_GOLD
    accent_bar.line.fill.background()

    # Title & Artist Textbox
    tb = slide.shapes.add_textbox(Inches(1.5), Inches(2.3), Inches(10.333), Inches(3.5))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = MSO_ANCHOR.MIDDLE

    # Optional Performance header
    if perf_title:
        p_perf = tf.paragraphs[0]
        p_perf.text = perf_title
        p_perf.font.name = "Arial"
        p_perf.font.size = Pt(20)
        p_perf.font.color.rgb = ACCENT_CYAN
        _apply_rtl_if_needed(p_perf, perf_title)
        p_title = tf.add_paragraph()
    else:
        p_title = tf.paragraphs[0]

    # Song Title
    song_title = song.get("title", "שיר ללא שם")
    p_title.text = song_title
    p_title.font.name = "Arial"
    p_title.font.size = Pt(56)
    p_title.font.bold = True
    p_title.font.color.rgb = TEXT_WHITE
    p_title.space_before = Pt(14)
    _apply_rtl_if_needed(p_title, song_title)

    # Artist
    artist = song.get("artist", "")
    if artist:
        p_artist = tf.add_paragraph()
        p_artist.text = artist
        p_artist.font.name = "Arial"
        p_artist.font.size = Pt(32)
        p_artist.font.color.rgb = ACCENT_GOLD
        p_artist.space_before = Pt(10)
        _apply_rtl_if_needed(p_artist, artist)

    return slide

def create_verse_slide(prs, song: Dict[str, Any], verse: Dict[str, Any], verse_index: int, total_verses: int):
    """Add a high-readability verse slide with song header and progress indicator."""
    slide = prs.slides.add_slide(prs.slide_layouts[6])
    _set_slide_background(slide, prs.slide_width, prs.slide_height)

    # Top Header: Song Title & Progress
    header_tb = slide.shapes.add_textbox(Inches(1.0), Inches(0.5), Inches(11.333), Inches(0.8))
    htf = header_tb.text_frame
    htf.word_wrap = True
    hp = htf.paragraphs[0]
    
    song_title = song.get("title", "")
    lang = detect_language(verse.get("text", "") + song_title)
    
    if lang == "he":
        progress_str = f"בית {verse_index} מתוך {total_verses}"
        header_text = f"{song_title}  •  {progress_str}"
        hp.alignment = PP_ALIGN.RIGHT
    else:
        progress_str = f"Verse {verse_index} of {total_verses}"
        header_text = f"{song_title}  •  {progress_str}"
        hp.alignment = PP_ALIGN.LEFT
        
    hp.text = header_text
    hp.font.name = "Arial"
    hp.font.size = Pt(18)
    hp.font.color.rgb = ACCENT_CYAN
    _apply_rtl_if_needed(hp, header_text)

    # Subtle divider line under header
    divider = slide.shapes.add_shape(
        MSO_SHAPE.RECTANGLE,
        Inches(1.0), Inches(1.3), Inches(11.333), Inches(0.02)
    )
    divider.fill.solid()
    divider.fill.fore_color.rgb = BORDER_COLOR
    divider.line.fill.background()

    # Verse Text Box (Centered and Large)
    verse_tb = slide.shapes.add_textbox(Inches(1.0), Inches(1.6), Inches(11.333), Inches(5.2))
    vtf = verse_tb.text_frame
    vtf.word_wrap = True
    vtf.vertical_anchor = MSO_ANCHOR.MIDDLE

    verse_text = verse.get("text", "").strip()
    lines = [l.strip() for l in verse_text.split("\n") if l.strip()]

    # Dynamic font sizing based on line count to guarantee it fits cleanly
    if len(lines) <= 2:
        font_size = 46
    elif len(lines) <= 4:
        font_size = 38
    elif len(lines) <= 6:
        font_size = 32
    else:
        font_size = 26

    for i, line in enumerate(lines):
        p = vtf.paragraphs[0] if i == 0 else vtf.add_paragraph()
        p.text = line
        p.font.name = "Arial"
        p.font.size = Pt(font_size)
        p.font.bold = True
        p.font.color.rgb = TEXT_WHITE
        p.line_spacing = 1.25
        _apply_rtl_if_needed(p, line)

    return slide

def generate_presentation(songs: List[Dict[str, Any]], title: str = "שירה בציבור", filename_prefix: str = "performance") -> str:
    """Generate a complete 16:9 widescreen presentation deck for a setlist or song."""
    PRESENTATIONS_DIR.mkdir(parents=True, exist_ok=True)
    prs = Presentation()
    prs.slide_width = Inches(13.333)  # 16:9 Widescreen
    prs.slide_height = Inches(7.5)

    for song in songs:
        verses = song.get("verses", [])
        # Song Title Slide
        create_song_title_slide(prs, song, perf_title=title)
        
        # Verse Slides
        total_verses = len(verses)
        for idx, verse in enumerate(verses, 1):
            create_verse_slide(prs, song, verse, idx, total_verses)

    # Save output file
    timestamp = int(time.time())
    clean_prefix = "".join([c if c.isalnum() else "_" for c in filename_prefix])[:30]
    out_filename = f"{clean_prefix}_{timestamp}.pptx"
    out_path = PRESENTATIONS_DIR / out_filename
    prs.save(str(out_path))

    return str(out_path)
