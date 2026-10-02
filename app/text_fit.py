import os
from functools import lru_cache
from pathlib import Path
from typing import List, Optional

from PIL import ImageFont

# Size the reference font is loaded at; measurements are divided by it to get em units
_REF_SIZE = 100
# Average bold sans-serif glyph width (in em), used only when no font file can be found
_FALLBACK_CHAR_WIDTH = 0.62
_FALLBACK_LINE_HEIGHT = 1.15
# Leave a little slack so renderer differences never push text past the edge
_SAFETY = 0.95

_BOLD_ARIAL_CANDIDATES = [
    Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts" / "arialbd.ttf",
    Path("/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
    Path("/Library/Fonts/Arial Bold.ttf"),
    Path("/usr/share/fonts/truetype/msttcorefonts/Arial_Bold.ttf"),
    # Liberation Sans is metric-compatible with Arial
    Path("/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf"),
    Path("/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf"),
]

@lru_cache(maxsize=1)
def _measure_font() -> Optional[ImageFont.FreeTypeFont]:
    """Load bold Arial (or a metric-compatible substitute) for text measurement."""
    for path in _BOLD_ARIAL_CANDIDATES:
        if path.exists():
            try:
                return ImageFont.truetype(str(path), _REF_SIZE)
            except OSError:
                continue
    return None

def text_width_em(line: str) -> float:
    """Width of a line of bold Arial text, in multiples of the font's em size."""
    font = _measure_font()
    if font is None:
        return len(line) * _FALLBACK_CHAR_WIDTH
    return font.getlength(line) / _REF_SIZE

def line_height_em() -> float:
    """Natural line height (ascent + descent) of bold Arial, in multiples of the em size."""
    font = _measure_font()
    if font is None:
        return _FALLBACK_LINE_HEIGHT
    ascent, descent = font.getmetrics()
    return (ascent + descent) / _REF_SIZE

def fit_font_size(blocks: List[List[str]], box_width: float, box_height: float,
                  line_height: float, max_size: float) -> float:
    """Largest em size at which every block of lines fits the box.

    Each block's widest line must fit box_width, and its lines stacked at
    line_height (in em) must fit box_height. Units are whatever the box uses (px, pt).
    """
    size = max_size
    for lines in blocks:
        if not lines:
            continue
        widest = max(text_width_em(line) for line in lines)
        if widest > 0:
            size = min(size, box_width * _SAFETY / widest)
        size = min(size, box_height * _SAFETY / (len(lines) * line_height))
    return size
