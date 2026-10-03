//! Performance colors.

use serde_json::Value;

use crate::py::Dict;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub bg_color: String,
    pub text_color: String,
    pub title_color: String,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            bg_color: "#0B1120".into(),    // Deep midnight
            text_color: "#FFFFFF".into(),  // High contrast white
            title_color: "#38BDF8".into(), // Vibrant sky
        }
    }
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Theme colors from a performance dict, falling back to defaults for missing or invalid values.
pub fn resolve_theme(source: Option<&Dict>) -> Theme {
    let mut theme = Theme::default();
    if let Some(src) = source {
        for (key, slot) in [("bg_color", &mut theme.bg_color), ("text_color", &mut theme.text_color), ("title_color", &mut theme.title_color)] {
            if let Some(Value::String(v)) = src.get(key) {
                if is_hex_color(v) {
                    *slot = v.to_uppercase();
                }
            }
        }
    }
    theme
}

/// #RRGGBB as (r, g, b).
pub fn hex_to_rgb(color: &str) -> (u8, u8, u8) {
    let c = |i: usize| u8::from_str_radix(&color[i..i + 2], 16).unwrap_or(0);
    (c(1), c(3), c(5))
}
