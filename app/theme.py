import re
from typing import Dict, Any, Optional, Tuple

# Default performance colors (hex #RRGGBB)
DEFAULT_THEME = {
    "bg_color": "#0B1120",     # Deep midnight
    "text_color": "#FFFFFF",   # High contrast white
    "title_color": "#38BDF8",  # Vibrant sky
}

_HEX_COLOR = re.compile(r"^#[0-9A-Fa-f]{6}$")

def resolve_theme(source: Optional[Dict[str, Any]] = None) -> Dict[str, str]:
    """Take theme colors from a performance dict, falling back to defaults for missing or invalid values."""
    theme = dict(DEFAULT_THEME)
    for key in theme:
        value = (source or {}).get(key)
        if isinstance(value, str) and _HEX_COLOR.match(value):
            theme[key] = value.upper()
    return theme

def hex_to_rgb(color: str) -> Tuple[int, int, int]:
    """Convert #RRGGBB to an (r, g, b) tuple."""
    return tuple(int(color[i:i + 2], 16) for i in (1, 3, 5))
