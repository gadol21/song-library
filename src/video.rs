//! Karaoke videos (the Python app's video_generator.py), without FFmpeg.
//!
//! The layout is still described as an ASS subtitle script on a 1280x720 canvas, built exactly as before; this module
//! then draws it itself the way libass drew it (Arial via HarfBuzz-style shaping, left-to-right base direction,
//! outline and shadow, scaled to 1920x1080), encodes H.264 with OpenH264 and AAC with FDK, and writes the MP4.
//!
//! The picture changes only when a subtitle appears or disappears, so each distinct screen is drawn once. A frame
//! rate gives a constant-frame-rate video (plays everywhere); None gives one frame per screen, held exactly as long
//! as it is shown (variable frame rate: fastest, but not every player handles it).
//!
//! Every screen is drawn over the same backdrop: the background color with music notes in the side margins (`notes.rs`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

use crate::audio::{self, AacEncoder};
use crate::config::paths;
use crate::mp4::{self, OutSample, OutTrack};
use crate::notes::{self, Seg};
use crate::py::{self, Dict};
use crate::storage::detect_language;
use crate::text_fit::{fit_font_size, line_height_em};
use crate::theme::{hex_to_rgb, resolve_theme, Theme};

pub const VIDEO_W: f64 = 1280.0; // the canvas the layout is designed on (also the subtitle script size)
pub const VIDEO_H: f64 = 720.0;
pub const OUTPUT_W: usize = 1920; // size of the rendered video; the canvas is scaled up to it
pub const OUTPUT_H: usize = 1080;
// Screen regions (px): title strip at top, next-verse preview at bottom, verse fills the middle
const HEADER_MARGIN_X: i64 = 40;
const HEADER_MARGIN_TOP: i64 = 30;
const HEADER_HEIGHT: f64 = 90.0;
const VERSE_MARGIN_X: i64 = 70;
const VERSE_HEIGHT: f64 = 460.0;
const NEXT_FONT_SIZE: i64 = 30;
const NEXT_MARGIN_BOTTOM: i64 = 40;
const MAX_HEADER_SIZE: f64 = 72.0;
const MAX_VERSE_SIZE: f64 = 160.0;
const MIN_VERSE_SIZE: i64 = 24;
pub const MIN_FRAME_RATE: f64 = 1.0;
pub const MAX_FRAME_RATE: f64 = 30.0;
/// Used when nothing is specified: a constant rate plays in every player
pub const DEFAULT_FRAME_RATE: f64 = 5.0;

/// None means a variable frame rate; otherwise a constant rate in frames per second within the allowed range.
pub fn check_frame_rate(frame_rate: Option<f64>) -> Result<Option<f64>, String> {
    match frame_rate {
        None => Ok(None),
        Some(rate) if (MIN_FRAME_RATE..=MAX_FRAME_RATE).contains(&rate) => Ok(Some(rate)),
        Some(_) => Err(format!("קצב הפריימים חייב להיות בין {} ל-{} פריימים לשנייה", MIN_FRAME_RATE as i64, MAX_FRAME_RATE as i64)),
    }
}

// ---------------------------------------------------------------- the subtitle script (as the Python app wrote it)

/// Seconds as an ASS timestamp H:MM:SS.cs
pub fn format_ass_time(seconds: f64) -> String {
    let h = (seconds / 3600.0).floor() as i64;
    let m = ((seconds % 3600.0) / 60.0).floor() as i64;
    let mut s = (seconds % 60.0).floor() as i64;
    let mut cs = py::round_int((seconds - seconds.trunc()) * 100.0);
    if cs >= 100 {
        s += 1;
        cs -= 100;
    }
    format!("{}:{:02}:{:02}.{:02}", h, m, s, cs)
}

/// #RRGGBB as ASS &HAABBGGRR (alpha 0 = opaque, 255 = transparent).
fn ass_color(hex: &str, alpha: u8) -> String {
    let (r, g, b) = hex_to_rgb(hex);
    format!("&H{:02X}{:02X}{:02X}{:02X}", alpha, b, g, r)
}

fn verse_lines(verse: &Value) -> Vec<String> {
    let text = verse.get("text").and_then(Value::as_str).unwrap_or("");
    py::strip(text).split('\n').map(py::strip).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// Largest ASS Fontsize at which every block of lines fits the box. libass treats Fontsize as the font's line
/// height (ascent + descent), not its em size, so fit in em units and convert.
fn fit_ass_font_size(blocks: &[Vec<String>], box_width: f64, box_height: f64, max_size: f64) -> i64 {
    let lh = line_height_em();
    let em = fit_font_size(blocks, box_width, box_height, lh, max_size / lh);
    (em * lh) as i64
}

fn start_of(v: &Value) -> f64 {
    v.get("start_time").filter(|t| py::truthy(Some(t))).and_then(py::float).unwrap_or(0.0)
}

/// The Advanced SubStation Alpha script describing the whole song.
pub fn generate_ass_subtitles(song: &Dict, total_duration: f64, theme: &Theme) -> String {
    let title = py::get_str(song, "title", "");
    let artist = py::get_str(song, "artist", "");
    let mut verses: Vec<Value> = match song.get("verses") {
        Some(Value::Array(v)) => v.clone(),
        _ => Vec::new(),
    };
    verses.sort_by(|a, b| start_of(a).partial_cmp(&start_of(b)).unwrap_or(std::cmp::Ordering::Equal));
    let header_text = if artist.is_empty() { title.to_string() } else { format!("{} - {}", title, artist) };

    // One font size for the whole song: the largest at which its longest verse still fits
    let header_size = fit_ass_font_size(&[vec![header_text.clone()]], VIDEO_W - 2.0 * HEADER_MARGIN_X as f64, HEADER_HEIGHT, MAX_HEADER_SIZE);
    let blocks: Vec<Vec<String>> = verses.iter().map(verse_lines).collect();
    let verse_size = MIN_VERSE_SIZE.max(fit_ass_font_size(&blocks, VIDEO_W - 2.0 * VERSE_MARGIN_X as f64, VERSE_HEIGHT, MAX_VERSE_SIZE));

    let title_col = ass_color(&theme.title_color, 0);
    let text_col = ass_color(&theme.text_color, 0);
    // Next-verse preview: text color, partly transparent so it reads as secondary
    let next_col = ass_color(&theme.text_color, 0x70);

    let mut lines = vec![
        "[Script Info]".to_string(),
        "Title: Karaoke Video".into(),
        "ScriptType: v4.00+".into(),
        "WrapStyle: 0".into(),
        "ScaledBorderAndShadow: yes".into(),
        format!("PlayResX: {}", VIDEO_W as i64),
        format!("PlayResY: {}", VIDEO_H as i64),
        String::new(),
        "[V4+ Styles]".into(),
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding".into(),
        // Title at the top center
        format!("Style: Header,Arial,{},{},&H00000000,&H00000000,&H80000000,1,0,0,0,100,100,0,0,1,2,1,8,{},{},{},1", header_size, title_col, HEADER_MARGIN_X, HEADER_MARGIN_X, HEADER_MARGIN_TOP),
        // Active verse: centered, as large as fits, with a dark outline
        format!("Style: ActiveVerse,Arial,{},{},&H00000000,&H00000000,&HA0000000,1,0,0,0,100,100,0,0,1,3,2,5,{},{},0,1", verse_size, text_col, VERSE_MARGIN_X, VERSE_MARGIN_X),
        // Next verse preview: bottom, smaller, faded text color
        format!("Style: NextVerse,Arial,{},{},&H00000000,&H00000000,&HA0000000,0,0,0,0,100,100,0,0,1,2,1,2,{},{},{},1", NEXT_FONT_SIZE, next_col, VERSE_MARGIN_X, VERSE_MARGIN_X, NEXT_MARGIN_BOTTOM),
        String::new(),
        "[Events]".into(),
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".into(),
        // Persistent title at top for the whole song
        format!("Dialogue: 0,0:00:00.00,{},Header,,0,0,0,,{}", format_ass_time(total_duration), header_text),
    ];

    for (idx, v) in verses.iter().enumerate() {
        let start = start_of(v).max(0.0);
        // End: until the next verse starts, or end_time, or start + 6s
        let end_time = v.get("end_time").filter(|t| py::truthy(Some(t))).and_then(py::float);
        let end = match end_time {
            Some(e) if e > start => e,
            _ if idx + 1 < verses.len() => {
                let next = &verses[idx + 1];
                let next_start = next.get("start_time").filter(|t| py::truthy(Some(t))).and_then(py::float).unwrap_or(start + 5.0);
                (start + 1.0).max(next_start - 0.2)
            }
            _ => total_duration.min(start + 6.0),
        };
        let formatted = verse_lines(v).join("\\N");
        let (start_s, end_s) = (format_ass_time(start), format_ass_time(end));
        lines.push(format!("Dialogue: 1,{},{},ActiveVerse,,0,0,0,,{}", start_s, end_s, formatted));
        if idx + 1 < verses.len() {
            let next_lines = verse_lines(&verses[idx + 1]);
            let first_line = next_lines.first().cloned().unwrap_or_default();
            let label = if detect_language(&first_line) == "he" { "הבא: " } else { "Next: " };
            lines.push(format!("Dialogue: 0,{},{},NextVerse,,0,0,0,,{}{}", start_s, end_s, label, first_line));
        }
    }
    lines.join("\n")
}

/// Seconds from an ASS timestamp (H:MM:SS.cs).
fn parse_ass_time(stamp: &str) -> f64 {
    let mut parts = stamp.splitn(3, ':');
    let h: f64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0.0);
    let m: f64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0.0);
    let s: f64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + s
}

// ---------------------------------------------------------------- the script, read back

#[derive(Clone)]
struct Style {
    name: String,
    size: f64,
    bold: bool,
    /// (r, g, b, ass alpha) for primary, outline and back (shadow) colors
    primary: (u8, u8, u8, u8),
    outline_color: (u8, u8, u8, u8),
    back: (u8, u8, u8, u8),
    outline: f64,
    shadow: f64,
    alignment: u8,
    margin_l: f64,
    margin_r: f64,
    margin_v: f64,
}

struct Event {
    layer: i64,
    order: usize,
    start: f64,
    end: f64,
    style: usize,
    text: String,
}

fn parse_color(s: &str) -> (u8, u8, u8, u8) {
    let v = u32::from_str_radix(s.trim_start_matches("&H").trim_start_matches("&h"), 16).unwrap_or(0);
    ((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8, ((v >> 24) & 0xFF) as u8)
}

fn parse_script(script: &str) -> (Vec<Style>, Vec<Event>) {
    let mut styles = Vec::new();
    let mut events = Vec::new();
    for line in script.lines() {
        if let Some(rest) = line.strip_prefix("Style: ") {
            let f: Vec<&str> = rest.split(',').collect();
            if f.len() < 23 {
                continue;
            }
            let num = |i: usize| f[i].trim().parse::<f64>().unwrap_or(0.0);
            styles.push(Style {
                name: f[0].to_string(),
                size: num(2),
                bold: f[7].trim() != "0",
                primary: parse_color(f[3]),
                outline_color: parse_color(f[5]),
                back: parse_color(f[6]),
                outline: num(16),
                shadow: num(17),
                alignment: num(18) as u8,
                margin_l: num(19),
                margin_r: num(20),
                margin_v: num(21),
            });
        } else if let Some(rest) = line.strip_prefix("Dialogue: ") {
            let f: Vec<&str> = rest.splitn(10, ',').collect();
            if f.len() < 10 {
                continue;
            }
            let style = styles.iter().position(|s| s.name == f[3]).unwrap_or(0);
            events.push(Event {
                layer: f[0].trim().parse().unwrap_or(0),
                order: events.len(),
                start: parse_ass_time(f[1]),
                end: parse_ass_time(f[2]),
                style,
                text: f[9].to_string(),
            });
        }
    }
    (styles, events)
}

/// ASS event text as lines: "{...}" override blocks are dropped, \N breaks lines, \n is a space (WrapStyle 0)
/// and \h a non-breaking space.
fn event_lines(text: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if let Some(close) = chars[i + 1..].iter().position(|&x| x == '}') {
                i += close + 2;
                continue;
            }
        }
        if c == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'N' => {
                    lines.push(String::new());
                    i += 2;
                    continue;
                }
                'n' => {
                    lines.last_mut().unwrap().push(' ');
                    i += 2;
                    continue;
                }
                'h' => {
                    lines.last_mut().unwrap().push('\u{a0}');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        lines.last_mut().unwrap().push(c);
        i += 1;
    }
    lines
}

// ---------------------------------------------------------------- fonts

pub struct Font {
    data: &'static [u8],
    /// OS/2 usWinAscent / usWinDescent (what libass uses for line metrics, like GDI)
    win_ascent: f64,
    win_descent: f64,
}

impl Font {
    fn load(path: &Path) -> Option<Font> {
        let data: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
        let face = ttf_parser::Face::parse(data, 0).ok()?;
        let (asc, desc) = match face.tables().os2 {
            Some(os2) if os2.windows_ascender() as i32 + os2.windows_descender() as i32 != 0 => {
                (os2.windows_ascender() as f64, os2.windows_descender().unsigned_abs() as f64)
            }
            _ => (face.ascender() as f64, (face.descender() as f64).abs()),
        };
        Some(Font { data, win_ascent: asc, win_descent: desc })
    }

    fn face(&self) -> rustybuzz::Face<'static> {
        rustybuzz::Face::from_slice(self.data, 0).expect("font parsed before")
    }
}

fn font_candidates(bold: bool) -> Vec<PathBuf> {
    if bold {
        return crate::text_fit::bold_arial_candidates();
    }
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    vec![
        windir.join("Fonts").join("arial.ttf"),
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf"),
        PathBuf::from("/Library/Fonts/Arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/msttcorefonts/Arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"),
        PathBuf::from("/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf"),
    ]
}

fn font(bold: bool) -> Result<&'static Font, String> {
    static BOLD: OnceLock<Option<Font>> = OnceLock::new();
    static REGULAR: OnceLock<Option<Font>> = OnceLock::new();
    let cell = if bold { &BOLD } else { &REGULAR };
    cell.get_or_init(|| font_candidates(bold).iter().find_map(|p| Font::load(p)))
        .as_ref()
        .ok_or_else(|| format!("Arial{} font not found", if bold { " Bold" } else { "" }))
}

// ---------------------------------------------------------------- text layout (as libass lays it out)

/// A shaped glyph placed on a line: glyph id and pen x, in font units.
struct Placed {
    glyph: u16,
    x: f64,
    dx: f64,
    dy: f64,
}

struct ShapedLine {
    glyphs: Vec<Placed>,
    width: f64,
}

/// Shape one line the way the Python app's libass did: glyphs in logical order, left to right, with no bidirectional
/// reordering (checked against libass output pixel by pixel), kerning off as libass leaves it unless the script asks.
fn shape_line(font: &Font, text: &str) -> ShapedLine {
    let face = font.face();
    let mut glyphs = Vec::new();
    let mut pen = 0.0;
    if text.is_empty() {
        return ShapedLine { glyphs, width: 0.0 };
    }
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(rustybuzz::Direction::LeftToRight);
    let kern_off = [rustybuzz::Feature::new(ttf_parser::Tag::from_bytes(b"kern"), 0, ..)];
    let out = rustybuzz::shape(&face, &kern_off, buffer);
    for (info, pos) in out.glyph_infos().iter().zip(out.glyph_positions()) {
        glyphs.push(Placed { glyph: info.glyph_id as u16, x: pen, dx: pos.x_offset as f64, dy: pos.y_offset as f64 });
        pen += pos.x_advance as f64;
    }
    ShapedLine { glyphs, width: pen }
}

/// Width of a line in font units.
fn line_width(font: &Font, text: &str) -> f64 {
    shape_line(font, text).width
}

/// libass's smart wrapping (WrapStyle 0) for a line wider than the box: break at the last space before the overflow,
/// then move words between neighboring lines while that evens out their lengths.
fn wrap_line(font: &Font, text: &str, max_units: f64) -> Vec<String> {
    if line_width(font, text) <= max_units || !text.contains(' ') {
        return vec![text.to_string()];
    }
    let words: Vec<&str> = text.split(' ').collect();
    // Greedy: a new line starts at the first breakable point after overflow
    let mut breaks: Vec<usize> = Vec::new(); // word index starting each line after the first
    let mut start = 0;
    for i in 1..=words.len() {
        if i == words.len() {
            break;
        }
        let candidate = words[start..=i].join(" ");
        if line_width(font, &candidate) >= max_units {
            breaks.push(i);
            start = i;
        }
    }
    // Rebalance neighboring lines
    let measure = |from: usize, to: usize| line_width(font, &words[from..to].join(" "));
    loop {
        let mut changed = false;
        let mut bounds: Vec<usize> = vec![0];
        bounds.extend(&breaks);
        bounds.push(words.len());
        for k in 0..breaks.len() {
            let (s1, s2, s3) = (bounds[k], bounds[k + 1], bounds[k + 2]);
            if s2 - s1 < 2 {
                continue; // moving the only word would merge lines
            }
            let (l1, l2) = (measure(s1, s2), measure(s2, s3));
            let (n1, n2) = (measure(s1, s2 - 1), measure(s2 - 1, s3));
            if (n1 - n2).abs() < (l1 - l2).abs() {
                breaks[k] -= 1;
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    let mut bounds: Vec<usize> = vec![0];
    bounds.extend(&breaks);
    bounds.push(words.len());
    bounds.windows(2).map(|w| words[w[0]..w[1]].join(" ")).collect()
}

// ---------------------------------------------------------------- drawing

struct PathSink<'a> {
    pb: &'a mut tiny_skia::PathBuilder,
    x: f32,
    y: f32,
    scale: f32,
}

impl ttf_parser::OutlineBuilder for PathSink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.x + x * self.scale, self.y - y * self.scale);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.x + x * self.scale, self.y - y * self.scale);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.x + x1 * self.scale, self.y - y1 * self.scale, self.x + x * self.scale, self.y - y * self.scale);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x2 * self.scale,
            self.y - y2 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// An RGB frame.
#[derive(Clone)]
pub struct Frame {
    pub rgb: Vec<u8>,
}

impl Frame {
    fn new(bg: (u8, u8, u8)) -> Frame {
        let mut rgb = vec![0u8; OUTPUT_W * OUTPUT_H * 3];
        for px in rgb.chunks_exact_mut(3) {
            px.copy_from_slice(&[bg.0, bg.1, bg.2]);
        }
        Frame { rgb }
    }

    /// Blend a color through a coverage mask placed at (left, top).
    fn blend(&mut self, mask: &[u8], mw: usize, mh: usize, left: i64, top: i64, color: (u8, u8, u8, u8)) {
        let opacity = (255 - color.3 as u32) as f32 / 255.0;
        if opacity <= 0.0 {
            return;
        }
        for my in 0..mh {
            let y = top + my as i64;
            if y < 0 || y >= OUTPUT_H as i64 {
                continue;
            }
            for mx in 0..mw {
                let m = mask[my * mw + mx];
                if m == 0 {
                    continue;
                }
                let x = left + mx as i64;
                if x < 0 || x >= OUTPUT_W as i64 {
                    continue;
                }
                let a = m as f32 / 255.0 * opacity;
                let p = (y as usize * OUTPUT_W + x as usize) * 3;
                for (c, v) in [color.0, color.1, color.2].iter().enumerate() {
                    let old = self.rgb[p + c] as f32;
                    self.rgb[p + c] = (old + (*v as f32 - old) * a).round() as u8;
                }
            }
        }
    }
}

/// Coverage mask of a path (filled, or stroked with round joins) within a box.
fn coverage(path: &tiny_skia::Path, stroke_width: Option<f32>, w: usize, h: usize, dx: f32, dy: f32) -> Vec<u8> {
    let mut mask = tiny_skia::Mask::new(w as u32, h as u32).expect("mask size");
    let transform = tiny_skia::Transform::from_translate(dx, dy);
    match stroke_width {
        None => mask.fill_path(path, tiny_skia::FillRule::Winding, true, transform),
        Some(width) => {
            let stroke = tiny_skia::Stroke { width, line_join: tiny_skia::LineJoin::Round, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
            if let Some(stroked) = path.stroke(&stroke, 1.0) {
                mask.fill_path(&stroked, tiny_skia::FillRule::Winding, true, transform);
            }
        }
    }
    mask.data().to_vec()
}

/// Draw one subtitle event onto the frame as libass does: shadow, then outline, then the text.
fn draw_event(frame: &mut Frame, style: &Style, text: &str) -> Result<(), String> {
    let font = font(style.bold)?;
    let scale = OUTPUT_H as f64 / VIDEO_H; // ScaledBorderAndShadow: borders scale with the canvas too
    let font_px = style.size * scale; // libass: Fontsize is the line height (win ascent + descent)
    let units_to_px = font_px / (font.win_ascent + font.win_descent);
    let asc = font.win_ascent * units_to_px;
    let line_h = font_px;
    let margin_l = style.margin_l * scale;
    let max_width_px = (VIDEO_W - style.margin_r) * scale - margin_l;

    let mut lines: Vec<String> = Vec::new();
    for l in event_lines(text) {
        lines.extend(wrap_line(font, &l, max_width_px / units_to_px));
    }
    if lines.iter().all(|l| l.trim().is_empty()) {
        return Ok(());
    }
    let shaped: Vec<ShapedLine> = lines.iter().map(|l| shape_line(font, l)).collect();
    let text_h = line_h * lines.len() as f64;
    let first_baseline = match (style.alignment.max(1) - 1) / 3 {
        // top (7, 8, 9)
        2 => style.margin_v * scale + asc,
        // middle (4, 5, 6)
        1 => VIDEO_H / 2.0 * scale - text_h / 2.0 + asc,
        // bottom (1, 2, 3)
        _ => (VIDEO_H - style.margin_v) * scale - text_h + asc,
    };

    // All glyph outlines of the event, positioned
    let face = ttf_parser::Face::parse(font.data, 0).map_err(|e| e.to_string())?;
    let mut pb = tiny_skia::PathBuilder::new();
    for (i, line) in shaped.iter().enumerate() {
        let width_px = line.width * units_to_px;
        // Numpad alignment: 1,4,7 left; 2,5,8 center; 3,6,9 right
        let x0 = match (style.alignment.max(1) - 1) % 3 {
            0 => margin_l,
            2 => margin_l + max_width_px - width_px,
            _ => margin_l + (max_width_px - width_px) / 2.0,
        };
        let baseline = first_baseline + i as f64 * line_h;
        for g in &line.glyphs {
            let mut sink = PathSink {
                pb: &mut pb,
                x: (x0 + (g.x + g.dx) * units_to_px) as f32,
                y: (baseline - g.dy * units_to_px) as f32,
                scale: units_to_px as f32,
            };
            face.outline_glyph(ttf_parser::GlyphId(g.glyph), &mut sink);
        }
    }
    let Some(path) = pb.finish() else { return Ok(()) };

    let border = style.outline * scale;
    let shadow = style.shadow * scale;
    let b = path.bounds();
    let pad = border + shadow + 3.0;
    let left = (b.left() as f64 - pad).floor() as i64;
    let top = (b.top() as f64 - pad).floor() as i64;
    let w = ((b.right() as f64 + pad).ceil() as i64 - left).max(1) as usize;
    let h = ((b.bottom() as f64 + pad).ceil() as i64 - top).max(1) as usize;
    let (dx, dy) = (-left as f32, -top as f32);

    let glyph_mask = coverage(&path, None, w, h, dx, dy);
    let mut outline_mask = glyph_mask.clone();
    if border > 0.0 {
        let ring = coverage(&path, Some((2.0 * border) as f32), w, h, dx, dy);
        for (o, r) in outline_mask.iter_mut().zip(ring) {
            *o = (*o).max(r);
        }
    }
    let primary_opaque = style.primary.3 == 0;
    if shadow > 0.0 && border > 0.0 {
        // Shadow: the outlined shape, offset
        let mut shadow_mask = coverage(&path, None, w, h, dx + shadow as f32, dy + shadow as f32);
        let ring = coverage(&path, Some((2.0 * border) as f32), w, h, dx + shadow as f32, dy + shadow as f32);
        for (s, r) in shadow_mask.iter_mut().zip(ring) {
            *s = (*s).max(r);
        }
        frame.blend(&shadow_mask, w, h, left, top, style.back);
    }
    if border > 0.0 {
        if !primary_opaque {
            // Semi-transparent text: the outline doesn't continue under the letters (libass fix_outline)
            for (o, g) in outline_mask.iter_mut().zip(&glyph_mask) {
                *o = if *o > *g { *o - *g / 2 } else { 0 };
            }
        }
        frame.blend(&outline_mask, w, h, left, top, style.outline_color);
    }
    frame.blend(&glyph_mask, w, h, left, top, style.primary);
    Ok(())
}

/// The background every screen is drawn on: the background color with the music notes around the edge.
pub fn backdrop(theme: &Theme) -> Frame {
    let mut frame = Frame::new(hex_to_rgb(&theme.bg_color));
    let (r, g, b) = hex_to_rgb(&notes::note_color(theme));
    let scale = (OUTPUT_H as f64 / VIDEO_H) as f32;
    for note in notes::frame_notes() {
        let (l, t, rt, bt) = notes::bounds(&note);
        let left = (l * scale as f64).floor() as i64 - 1;
        let top = (t * scale as f64).floor() as i64 - 1;
        let w = ((rt * scale as f64).ceil() as i64 - left + 2) as usize;
        let h = ((bt * scale as f64).ceil() as i64 - top + 2) as usize;
        let (dx, dy) = (-left as f32, -top as f32);
        // Each part filled on its own, then united, so overlaps keep one color
        let mut mask = vec![0u8; w * h];
        for part in &note {
            let mut pb = tiny_skia::PathBuilder::new();
            for seg in part {
                match *seg {
                    Seg::Move(x, y) => pb.move_to(x as f32 * scale, y as f32 * scale),
                    Seg::Line(x, y) => pb.line_to(x as f32 * scale, y as f32 * scale),
                    Seg::Cubic(x1, y1, x2, y2, x, y) => pb.cubic_to(
                        x1 as f32 * scale,
                        y1 as f32 * scale,
                        x2 as f32 * scale,
                        y2 as f32 * scale,
                        x as f32 * scale,
                        y as f32 * scale,
                    ),
                }
            }
            pb.close();
            if let Some(path) = pb.finish() {
                for (m, c) in mask.iter_mut().zip(coverage(&path, None, w, h, dx, dy)) {
                    *m = (*m).max(c);
                }
            }
        }
        frame.blend(&mask, w, h, left, top, (r, g, b, 0));
    }
    frame
}

/// The distinct screens of a script: change times, and per screen the events on it (in drawing order).
pub struct ScreenPlan {
    pub times: Vec<f64>,
    styles: Vec<Style>,
    events: Vec<Event>,
}

impl ScreenPlan {
    pub fn new(script: &str) -> ScreenPlan {
        let (styles, mut events) = parse_script(script);
        let mut times: Vec<f64> = events.iter().flat_map(|e| [e.start, e.end]).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times.dedup();
        events.sort_by_key(|e| (e.layer, e.order));
        ScreenPlan { times, styles, events }
    }

    pub fn count(&self) -> usize {
        self.times.len().saturating_sub(1)
    }

    /// Draw screen i (shown from times[i] to times[i + 1]) over the backdrop.
    pub fn render(&self, i: usize, backdrop: &Frame) -> Result<Frame, String> {
        let mut frame = backdrop.clone();
        let t = self.times[i];
        for e in &self.events {
            if e.start <= t && t < e.end {
                draw_event(&mut frame, &self.styles[e.style], &e.text)?;
            }
        }
        Ok(frame)
    }
}

// ---------------------------------------------------------------- encoding

/// BT.601 limited-range 4:2:0, as FFmpeg converted the drawn screens.
fn to_yuv420(frame: &Frame) -> Vec<u8> {
    let (w, h) = (OUTPUT_W, OUTPUT_H);
    let mut out = vec![0u8; w * h * 3 / 2];
    let (y_plane, uv) = out.split_at_mut(w * h);
    let (u_plane, v_plane) = uv.split_at_mut(w * h / 4);
    let rgb = &frame.rgb;
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) * 3;
            let (r, g, b) = (rgb[p] as f32, rgb[p + 1] as f32, rgb[p + 2] as f32);
            y_plane[y * w + x] = (16.0 + (65.481 * r + 128.553 * g + 24.966 * b) / 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    for cy in 0..h / 2 {
        for cx in 0..w / 2 {
            let mut sum = [0f32; 3];
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let p = ((cy * 2 + dy) * w + cx * 2 + dx) * 3;
                for c in 0..3 {
                    sum[c] += rgb[p + c] as f32;
                }
            }
            let (r, g, b) = (sum[0] / 4.0, sum[1] / 4.0, sum[2] / 4.0);
            u_plane[cy * (w / 2) + cx] = (128.0 + (-37.797 * r - 74.203 * g + 112.0 * b) / 255.0).round().clamp(0.0, 255.0) as u8;
            v_plane[cy * (w / 2) + cx] = (128.0 + (112.0 * r - 93.786 * g - 18.214 * b) / 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

struct VideoEncoder {
    encoder: openh264::encoder::Encoder,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

impl VideoEncoder {
    fn new(keyframe_interval: u32) -> Result<VideoEncoder, String> {
        use openh264::encoder::*;
        let config = EncoderConfig::new()
            .skip_frames(false)
            .rate_control_mode(RateControlMode::Quality)
            .bitrate(BitRate::from_bps(8_000_000))
            .qp(QpRange::new(18, 26))
            .usage_type(UsageType::ScreenContentRealTime)
            .intra_frame_period(IntraFramePeriod::from_num_frames(keyframe_interval))
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            .num_threads(0)  // 0 = auto: use all cores
            .vui(VuiConfig::bt601());
        let encoder = Encoder::with_api_config(openh264::OpenH264API::from_source(), config).map_err(|e| e.to_string())?;
        Ok(VideoEncoder { encoder, sps: Vec::new(), pps: Vec::new() })
    }

    /// Encode one frame; returns (length-prefixed NAL units, is keyframe).
    fn encode(&mut self, yuv: &[u8]) -> Result<(Vec<u8>, bool), String> {
        let src = openh264::formats::YUVSlices::new(
            (&yuv[..OUTPUT_W * OUTPUT_H], &yuv[OUTPUT_W * OUTPUT_H..OUTPUT_W * OUTPUT_H * 5 / 4], &yuv[OUTPUT_W * OUTPUT_H * 5 / 4..]),
            (OUTPUT_W, OUTPUT_H),
            (OUTPUT_W, OUTPUT_W / 2, OUTPUT_W / 2),
        );
        let bs = self.encoder.encode(&src).map_err(|e| e.to_string())?;
        let key = matches!(bs.frame_type(), openh264::encoder::FrameType::IDR);
        let mut sample = Vec::new();
        for l in 0..bs.num_layers() {
            let layer = bs.layer(l).unwrap();
            for n in 0..layer.nal_count() {
                let nal = layer.nal_unit(n).unwrap();
                let start = if nal.starts_with(&[0, 0, 0, 1]) { 4 } else if nal.starts_with(&[0, 0, 1]) { 3 } else { 0 };
                let body = &nal[start..];
                if body.is_empty() {
                    continue;
                }
                match body[0] & 0x1F {
                    7 => self.sps = body.to_vec(),
                    8 => self.pps = body.to_vec(),
                    _ => {
                        sample.extend_from_slice(&(body.len() as u32).to_be_bytes());
                        sample.extend_from_slice(body);
                    }
                }
            }
        }
        Ok((sample, key))
    }
}

/// Encoded tracks being assembled (a song, or a whole performance one song after another).
struct Movie {
    fps: Option<f64>,
    video: VideoEncoder,
    video_timescale: u32,
    video_samples: Vec<OutSample>,
    /// Video time written so far, in the video timescale
    video_time: u64,
    pcm: Vec<f32>,
    audio_rate: u32,
    audio_channels: usize,
}

const VFR_TIMESCALE: u32 = 90_000;

impl Movie {
    fn new(fps: Option<f64>, audio_rate: u32, audio_channels: usize) -> Result<Movie, String> {
        let (keyint, timescale) = match fps {
            // x264's default keyframe interval; one tick per thousandth of a frame keeps every frame the same length
            Some(f) => (250, (f * 1000.0).round() as u32),
            // -g 30: a keyframe at least every 30 screens, so seeking in a player stays quick
            None => (30, VFR_TIMESCALE),
        };
        Ok(Movie {
            fps,
            video: VideoEncoder::new(keyint)?,
            video_timescale: timescale,
            video_samples: Vec::new(),
            video_time: 0,
            pcm: Vec::new(),
            audio_rate,
            audio_channels,
        })
    }

    fn push_frame(&mut self, yuv: &[u8], duration: u64) -> Result<(), String> {
        if duration == 0 {
            return Ok(());
        }
        let (data, sync) = self.video.encode(yuv)?;
        if data.is_empty() {
            // A skipped frame: the previous picture just lasts longer
            if let Some(last) = self.video_samples.last_mut() {
                last.duration += duration as u32;
                self.video_time += duration;
                return Ok(());
            }
        }
        self.video_samples.push(OutSample { data, duration: duration as u32, cts: 0, sync });
        self.video_time += duration;
        Ok(())
    }

    /// Add one song: its screens for `length` seconds and its audio (padded or cut to the same length).
    fn add_song(&mut self, plan: &ScreenPlan, backdrop: &Frame, length: f64, pcm: Option<&audio::Pcm>) -> Result<(), String> {
        let count = plan.count();
        let segment_start = self.video_time;
        match self.fps {
            Some(fps) => {
                // Each change moves to the first frame at or after it, so every screen lasts whole frames
                let first_frame: Vec<i64> = plan.times.iter().map(|t| (py::round_to(t * fps, 6)).ceil() as i64).collect();
                let frames = ((length * fps) - 1e-9).ceil().max(1.0) as i64;
                let mut current: Option<(usize, Vec<u8>)> = None;
                for n in 0..frames {
                    let screen = (0..count).rev().find(|&i| first_frame[i] <= n && first_frame[i + 1] > first_frame[i]).unwrap_or(0);
                    let screen = if count == 0 { usize::MAX } else { screen };
                    if current.as_ref().map(|c| c.0) != Some(screen) {
                        let frame = if screen == usize::MAX { backdrop.clone() } else { plan.render(screen, backdrop)? };
                        current = Some((screen, to_yuv420(&frame)));
                    }
                    self.push_frame(&current.as_ref().unwrap().1, 1000)?;
                }
            }
            None => {
                // One frame per screen, held exactly as long as it is shown. As with FFmpeg's concat list, every change
                // lands on the next 1/25 s tick, and the last screen is repeated for one more tick so its length is recorded.
                let scale = self.video_timescale as f64;
                let tick = |t: f64| ((py::round_to(t * 25.0, 6)).ceil() / 25.0 * scale).round() as u64;
                let mut last_frame: Option<Vec<u8>> = None;
                for i in 0..count {
                    let (from, to) = (tick(plan.times[i]), tick(plan.times[i + 1]));
                    if to <= from {
                        continue;
                    }
                    let yuv = to_yuv420(&plan.render(i, backdrop)?);
                    self.push_frame(&yuv, to - from)?;
                    last_frame = Some(yuv);
                }
                let yuv = last_frame.unwrap_or_else(|| to_yuv420(backdrop));
                self.push_frame(&yuv, (self.video_timescale / 25) as u64)?;
            }
        }
        // Audio covers the same span as this song's video
        let video_secs = (self.video_time - segment_start) as f64 / self.video_timescale as f64;
        let want = (video_secs * self.audio_rate as f64).round() as usize * self.audio_channels;
        let start = self.pcm.len();
        if let Some(p) = pcm {
            let converted = convert_pcm(p, self.audio_rate, self.audio_channels);
            self.pcm.extend_from_slice(&converted[..converted.len().min(want)]);
        }
        self.pcm.resize(start + want, 0.0);
        Ok(())
    }

    fn finish(self, out_path: &Path, audio_bitrate: u32) -> Result<(), String> {
        let mut aac = AacEncoder::new(self.audio_rate, self.audio_channels, audio_bitrate)?;
        let frames = self.pcm.len() / self.audio_channels;
        let units = aac.encode_all(&self.pcm)?;
        let audio_samples: Vec<OutSample> = units.into_iter().map(|data| OutSample { data, duration: 1024, cts: 0, sync: true }).collect();
        let video = OutTrack {
            handler: *b"vide",
            timescale: self.video_timescale,
            sample_entry: mp4::avc1_entry(OUTPUT_W as u16, OUTPUT_H as u16, &self.video.sps, &self.video.pps),
            width: OUTPUT_W as u32,
            height: OUTPUT_H as u32,
            edits: Vec::new(),
            samples: self.video_samples,
        };
        // The edit list skips the encoder's priming samples, so sound and picture start together
        let audio_ms = (frames as u64 * 1000 + self.audio_rate as u64 / 2) / self.audio_rate as u64;
        let audio = OutTrack {
            handler: *b"soun",
            timescale: self.audio_rate,
            sample_entry: mp4::mp4a_entry(self.audio_channels as u16, self.audio_rate, &aac.config, audio_bitrate),
            width: 0,
            height: 0,
            edits: vec![(audio_ms, aac.delay as i64)],
            samples: audio_samples,
        };
        mp4::write_movie(out_path, &[video, audio])
    }
}

/// Audio resampled (linear) and remixed to the movie's format, for a performance whose songs differ.
fn convert_pcm(p: &audio::Pcm, rate: u32, channels: usize) -> Vec<f32> {
    let p_channels = p.channels.max(1);
    let frames_in = p.samples.len() / p_channels;
    let pick = |frame: usize, ch: usize| -> f32 {
        let base = frame * p_channels;
        if channels == p_channels {
            p.samples[base + ch]
        } else if channels == 1 {
            (0..p_channels.min(2)).map(|c| p.samples[base + c]).sum::<f32>() / p_channels.min(2) as f32
        } else {
            p.samples[base + ch.min(p_channels - 1)]
        }
    };
    if p.rate == rate && channels == p_channels {
        return p.samples.clone();
    }
    let frames_out = (frames_in as f64 * rate as f64 / p.rate as f64).round() as usize;
    let mut out = Vec::with_capacity(frames_out * channels);
    for n in 0..frames_out {
        let pos = n as f64 * p.rate as f64 / rate as f64;
        let i = pos.floor() as usize;
        let frac = (pos - i as f64) as f32;
        for ch in 0..channels {
            let a = if i < frames_in { pick(i, ch) } else { 0.0 };
            let b = if i + 1 < frames_in { pick(i + 1, ch) } else { a };
            out.push(a + (b - a) * frac);
        }
    }
    out
}

/// A song's audio (decoded) and length: the audio file's length as ffprobe reported it, else the song's stored
/// duration, else derived from the verses.
fn song_audio(song: &Dict) -> (Option<audio::Pcm>, f64) {
    let audio_path = song.get("audio_path").and_then(Value::as_str).map(PathBuf::from).filter(|p| p.exists());
    let mut duration = song.get("duration").and_then(py::float).unwrap_or(0.0);
    let mut pcm = None;
    if let Some(path) = &audio_path {
        let probed = audio::probe_duration(path);
        if probed > 0.0 {
            duration = probed;
        }
        match audio::decode(path) {
            Ok(p) => pcm = Some(audio::to_stereo_or_mono(p)),
            Err(e) => crate::log::warn(&format!("Audio of {} could not be decoded: {}", path.display(), e)),
        }
    }
    if duration <= 0.0 {
        let verses = match song.get("verses") {
            Some(Value::Array(v)) => v.clone(),
            _ => Vec::new(),
        };
        duration = if verses.is_empty() {
            15.0
        } else {
            verses
                .iter()
                .map(|v| v.get("end_time").filter(|e| py::truthy(Some(e))).and_then(py::float).unwrap_or_else(|| start_of(v) + 5.0))
                .fold(f64::NEG_INFINITY, f64::max)
                + 3.0
        };
    }
    (pcm, duration)
}

fn output_path(prefix: &str) -> Result<PathBuf, String> {
    let dir = &paths().videos;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{}_{}.mp4", py::clean_prefix(prefix), py::time_now() as i64)))
}

/// Render several songs one after another into one MP4.
fn render_songs(songs: &[Dict], theme: &Theme, frame_rate: Option<f64>, out_path: &Path) -> Result<(), String> {
    let prepared: Vec<(Option<audio::Pcm>, f64)> = songs.iter().map(song_audio).collect();
    // One audio format for the whole file: the first song's, else 44.1 kHz stereo (as the silent track was)
    let (rate, channels, bitrate) = match prepared.iter().find_map(|(p, _)| p.as_ref()) {
        Some(p) => (p.rate, p.channels, 192_000),
        None => (44_100, 2, 128_000),
    };
    let mut movie = Movie::new(frame_rate, rate, channels)?;
    let backdrop = backdrop(theme);
    for (song, (pcm, duration)) in songs.iter().zip(&prepared) {
        let script = generate_ass_subtitles(song, *duration, theme);
        let plan = ScreenPlan::new(&script);
        // In a performance each song's part runs a moment past its end in variable-frame-rate mode, as before
        let length = *duration;
        movie.add_song(&plan, &backdrop, length, pcm.as_ref())?;
    }
    movie.finish(out_path, bitrate)
}

/// Render a 1080p MP4 karaoke video for a song; returns its path.
pub fn generate_karaoke_video(song: &Dict, filename_prefix: &str, theme: Option<&Dict>, frame_rate: Option<f64>) -> Result<PathBuf, String> {
    let frame_rate = check_frame_rate(frame_rate)?;
    let theme = resolve_theme(theme);
    let id = py::get_str(song, "id", filename_prefix);
    let out = output_path(id)?;
    render_songs(std::slice::from_ref(song), &theme, frame_rate, &out)?;
    Ok(out)
}

/// Render one MP4 with all songs of a performance, one after another; returns its path.
pub fn generate_performance_video(songs: &[Dict], filename_prefix: &str, theme: Option<&Dict>, frame_rate: Option<f64>) -> Result<PathBuf, String> {
    let frame_rate = check_frame_rate(frame_rate)?;
    if songs.is_empty() {
        return Err("No songs provided for performance video".into());
    }
    if songs.len() == 1 {
        return generate_karaoke_video(&songs[0], filename_prefix, theme, frame_rate);
    }
    let theme = resolve_theme(theme);
    let out = output_path(filename_prefix)?;
    render_songs(songs, &theme, frame_rate, &out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_times() {
        assert_eq!(format_ass_time(1.5), "0:00:01.50");
        assert_eq!(format_ass_time(22.0), "0:00:22.00");
        assert_eq!(format_ass_time(3725.999), "1:02:06.00");
        assert_eq!(parse_ass_time("0:01:13.40"), 73.4);
    }

    #[test]
    fn event_text() {
        assert_eq!(event_lines("a\\Nb{\\i1}c\\hd"), vec!["a", "bc\u{a0}d"]);
    }
}
