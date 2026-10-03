//! Font sizing shared by the PowerPoint and video exports: the largest size at which text fits a box.
//!
//! Text is measured exactly as the Python app measured it with Pillow (FreeType, basic layout) on bold Arial
//! at 100 px, so the same songs get the same font sizes:
//! - each glyph's advance is its hinted width at 100 ppem, i.e. the font-unit advance scaled and rounded to whole
//!   pixels (a composite glyph flagged USE_MY_METRICS takes its component's advance);
//! - kerning from the 'kern' table is rounded to whole pixels, and Pillow then adds that pixel count to a 1/64 px
//!   value, so a kerning pair contributes kern_px / 64 px. (A Pillow quirk, kept for identical results.)
//! Verified against Pillow 12.3 for every glyph and kerning pair of Windows' arialbd.ttf.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Size the reference font is measured at; measurements are divided by it to get em units
const REF_SIZE: i64 = 100;
/// Average bold sans-serif glyph width (in em), used only when no font file can be found
const FALLBACK_CHAR_WIDTH: f64 = 0.62;
const FALLBACK_LINE_HEIGHT: f64 = 1.15;
/// Leave a little slack so renderer differences never push text past the edge
const SAFETY: f64 = 0.95;

pub fn bold_arial_candidates() -> Vec<PathBuf> {
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    vec![
        windir.join("Fonts").join("arialbd.ttf"),
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
        PathBuf::from("/Library/Fonts/Arial Bold.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/msttcorefonts/Arial_Bold.ttf"),
        // Liberation Sans is metric-compatible with Arial
        PathBuf::from("/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf"),
        PathBuf::from("/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf"),
    ]
}

/// Bold Arial (or a metric-compatible substitute) reduced to what measuring needs.
struct MeasureFont {
    /// Unicode -> glyph index
    cmap: std::collections::HashMap<u32, u16>,
    /// Hinted advance in 26.6 pixels at REF_SIZE, by glyph index
    advance: Vec<i64>,
    /// Kerning pairs in font units, from the 'kern' table
    kern: std::collections::HashMap<(u16, u16), i16>,
    x_scale: i64,
    ascent: i64,
    descent: i64,
}

/// FreeType's FT_MulFix: (a * b) / 0x10000, rounded.
fn mul_fix(a: i64, b: i64) -> i64 {
    let sign = if (a < 0) != (b < 0) { -1 } else { 1 };
    let (a, b) = (a.abs(), b.abs());
    sign * ((a * b + 0x8000) >> 16)
}

fn pix_round(x: i64) -> i64 {
    (x + 32) & -64
}

fn pix_ceil(x: i64) -> i64 {
    (x + 63) & -64
}

fn pix_floor(x: i64) -> i64 {
    x & -64
}

/// Pillow's PIXEL(): 26.6 to whole pixels, rounded
fn pixel(x: i64) -> i64 {
    ((x + 32) & -64) >> 6
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

impl MeasureFont {
    fn load(data: &[u8]) -> Option<MeasureFont> {
        let face = ttf_parser::Face::parse(data, 0).ok()?;
        let upem = face.units_per_em() as i64;
        let x_scale = ((REF_SIZE * 64) << 16) / upem;
        let raw = face.raw_face();

        // Composite glyphs whose component is flagged USE_MY_METRICS take that component's advance
        let loca_long = be16(raw.table(ttf_parser::Tag::from_bytes(b"head"))?, 50)? == 1;
        let loca = raw.table(ttf_parser::Tag::from_bytes(b"loca"));
        let glyf = raw.table(ttf_parser::Tag::from_bytes(b"glyf"));
        let glyph_range = |g: u16| -> Option<(usize, usize)> {
            let loca = loca?;
            let (a, b) = if loca_long {
                (be32(loca, g as usize * 4)? as usize, be32(loca, g as usize * 4 + 4)? as usize)
            } else {
                (be16(loca, g as usize * 2)? as usize * 2, be16(loca, g as usize * 2 + 2)? as usize * 2)
            };
            Some((a, b))
        };
        let metrics_glyph = |mut g: u16| -> u16 {
            for _ in 0..8 {
                let (Some(glyf), Some((start, end))) = (glyf, glyph_range(g)) else { return g };
                if end <= start || be16(glyf, start).map(|n| n as i16) >= Some(0) {
                    return g;
                }
                let mut at = start + 10;
                let mut next = None;
                loop {
                    let (Some(flags), Some(component)) = (be16(glyf, at), be16(glyf, at + 2)) else { break };
                    if flags & 0x0200 != 0 {
                        next = Some(component);
                        break;
                    }
                    at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
                    at += if flags & 0x0008 != 0 { 2 } else if flags & 0x0040 != 0 { 4 } else if flags & 0x0080 != 0 { 8 } else { 0 };
                    if flags & 0x0020 == 0 {
                        break;
                    }
                }
                match next {
                    Some(c) => g = c,
                    None => return g,
                }
            }
            g
        };

        let count = face.number_of_glyphs();
        let advance = (0..count)
            .map(|g| {
                let units = face.glyph_hor_advance(ttf_parser::GlyphId(metrics_glyph(g))).unwrap_or(0) as i64;
                pix_round(mul_fix(units, x_scale))
            })
            .collect();

        let mut cmap = std::collections::HashMap::new();
        if let Some(table) = face.tables().cmap {
            for sub in table.subtables {
                if !sub.is_unicode() {
                    continue;
                }
                sub.codepoints(|cp| {
                    if let Some(g) = sub.glyph_index(cp) {
                        cmap.entry(cp).or_insert(g.0);
                    }
                });
            }
        }

        let mut kern = std::collections::HashMap::new();
        if let Some(table) = face.tables().kern {
            // FreeType's FT_Get_Kerning reads the first horizontal format-0 subtable
            if let Some(sub) = table.subtables.into_iter().find(|s| s.horizontal && !s.variable) {
                if let ttf_parser::kern::Format::Format0(pairs) = sub.format {
                    for i in 0..pairs.pairs.len() {
                        if let Some(p) = pairs.pairs.get(i) {
                            kern.insert((p.left().0, p.right().0), p.value);
                        }
                    }
                }
            }
        }

        let hhea_asc = face.ascender() as i64;
        let hhea_desc = face.descender() as i64;
        let ascent = pix_ceil(mul_fix(hhea_asc, x_scale)) >> 6;
        let descent = -(pix_floor(mul_fix(hhea_desc, x_scale)) >> 6);
        Some(MeasureFont { cmap, advance, kern, x_scale, ascent, descent })
    }

    /// Pillow's ImageFont.getlength(line) at REF_SIZE.
    fn length(&self, line: &str) -> f64 {
        let mut total: i64 = 0;
        let mut last: u16 = 0;
        let mut kern_px_total: i64 = 0;
        for ch in line.chars() {
            let g = self.cmap.get(&(ch as u32)).copied().unwrap_or(0);
            if last != 0 && g != 0 {
                if let Some(&k) = self.kern.get(&(last, g)) {
                    kern_px_total += pixel(pix_round(mul_fix(k as i64, self.x_scale)));
                }
            }
            total += self.advance.get(g as usize).copied().unwrap_or(0);
            last = g;
        }
        (total + kern_px_total) as f64 / 64.0
    }
}

fn measure_font() -> Option<&'static MeasureFont> {
    static FONT: OnceLock<Option<MeasureFont>> = OnceLock::new();
    FONT.get_or_init(|| {
        for path in bold_arial_candidates() {
            if let Ok(data) = std::fs::read(&path) {
                if let Some(font) = MeasureFont::load(&data) {
                    return Some(font);
                }
            }
        }
        None
    })
    .as_ref()
}

/// Width of a line of bold Arial text, in multiples of the font's em size.
pub fn text_width_em(line: &str) -> f64 {
    match measure_font() {
        Some(f) => f.length(line) / REF_SIZE as f64,
        None => line.chars().count() as f64 * FALLBACK_CHAR_WIDTH,
    }
}

/// Natural line height (ascent + descent) of bold Arial, in multiples of the em size.
pub fn line_height_em() -> f64 {
    match measure_font() {
        Some(f) => (f.ascent + f.descent) as f64 / REF_SIZE as f64,
        None => FALLBACK_LINE_HEIGHT,
    }
}

/// Largest em size at which every block of lines fits the box.
///
/// Each block's widest line must fit box_width, and its lines stacked at line_height (in em) must fit
/// box_height. Units are whatever the box uses (px, pt).
pub fn fit_font_size(blocks: &[Vec<String>], box_width: f64, box_height: f64, line_height: f64, max_size: f64) -> f64 {
    let mut size = max_size;
    for lines in blocks {
        if lines.is_empty() {
            continue;
        }
        let widest = lines.iter().map(|l| text_width_em(l)).fold(f64::NEG_INFINITY, f64::max);
        if widest > 0.0 {
            size = size.min(box_width * SAFETY / widest);
        }
        size = size.min(box_height * SAFETY / (lines.len() as f64 * line_height));
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_pillow() {
        if measure_font().is_none() {
            return;
        }
        // Values printed by Pillow 12.3 for arialbd.ttf at 100 px
        assert_eq!(text_width_em("A") * 100.0, 72.0);
        assert_eq!(text_width_em("AV") * 100.0, 138.890625);
        assert_eq!(text_width_em("A V") * 100.0, 166.9375);
        assert_eq!(text_width_em("שלום") * 100.0, 209.0);
        assert_eq!(text_width_em("עוד לא תמו כל פלאייך, Hello AV To! 123 \u{200f}בית 3\u{200f} מתוך 7\u{200f}  •  ") * 100.0, 2463.71875);
        assert_eq!(line_height_em(), 1.13);
    }
}
