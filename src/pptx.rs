//! 16:9 PowerPoint decks for a song or a setlist (the Python app's pptx_generator.py).
//!
//! The package is the one python-pptx produced: its template parts are embedded verbatim (assets/pptx) and the
//! slides are written as the same XML. A song with audio gets it embedded; if its verses are timed the slides
//! also advance by themselves, each shown for exactly its verse, and the audio starts playing on its own.
//! Every slide has decorative music notes in its side margins (`notes.rs`, the same ones the video shows).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::audio;
use crate::notes::{self, Seg};
use crate::config::paths;
use crate::py::{self, Dict};
use crate::storage::detect_language;
use crate::text_fit::{fit_font_size, line_height_em};
use crate::theme::{resolve_theme, Theme};

// Text box geometry (inches). python-pptx text boxes have 0.1in side and 0.05in top/bottom insets.
const INSET_X: f64 = 0.1;
const INSET_Y: f64 = 0.05;
const TITLE_BOX: (f64, f64, f64, f64) = (1.5, 2.3, 10.333, 3.5); // left, top, width, height
const HEADER_BOX: (f64, f64, f64, f64) = (1.0, 0.5, 11.333, 0.8);
const VERSE_BOX: (f64, f64, f64, f64) = (1.0, 1.6, 11.333, 5.2);
const VERSE_LINE_SPACING: f64 = 1.25;
// Font size limits (pt)
const MAX_TITLE_PT: f64 = 96.0;
const MAX_HEADER_PT: f64 = 40.0;
const MAX_VERSE_PT: f64 = 110.0;
const MIN_VERSE_PT: f64 = 20.0;
/// Vertical room the song title may use on its slide (the rest is for the show name and artist)
const TITLE_HEIGHT_IN: f64 = 2.0;

/// PowerPoint plays only these formats; anything else is converted to MP3 first.
const AUDIO_CONTENT_TYPES: [(&str, &str); 3] = [(".mp3", "audio/mpeg"), (".m4a", "audio/mp4"), (".wav", "audio/wav")];
/// Seconds the title slide is shown when the first verse starts at (almost) 0, so the audio starts on the verse slide
const LEAD_IN_SECONDS: f64 = 3.0;
const MIN_TITLE_SECONDS: f64 = 2.0;
const LAST_VERSE_FALLBACK_SECONDS: f64 = 6.0;

const SLIDE_WIDTH: i64 = 12191695; // Inches(13.333)
const SLIDE_HEIGHT: i64 = 6858000; // Inches(7.5)

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const XML_DECL: &str = "<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\n";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const RT_LAYOUT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
const RT_AUDIO: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/audio";
const RT_MEDIA: &str = "http://schemas.microsoft.com/office/2007/relationships/media";
const RT_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const RT_SLIDE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

/// The parts python-pptx's template contributed unchanged, in the order it wrote them.
macro_rules! asset {
    ($p:literal) => {
        ($p, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pptx/", $p)) as &[u8])
    };
}
const HEAD_PARTS: &[(&str, &[u8])] = &[asset!("docProps/core.xml"), asset!("docProps/app.xml")];
const PRESENTATION_XML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pptx/ppt/presentation.xml"));
const TEMPLATE_PARTS: &[(&str, &[u8])] = &[
    asset!("ppt/presProps.xml"),
    asset!("ppt/viewProps.xml"),
    asset!("ppt/theme/theme1.xml"),
    asset!("ppt/tableStyles.xml"),
    asset!("ppt/slideMasters/slideMaster1.xml"),
    asset!("ppt/slideMasters/_rels/slideMaster1.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout11.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout11.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout1.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout1.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout2.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout2.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout3.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout3.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout4.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout4.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout5.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout5.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout6.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout6.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout7.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout7.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout8.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout8.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout9.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout9.xml.rels"),
    asset!("ppt/slideLayouts/slideLayout10.xml"),
    asset!("ppt/slideLayouts/_rels/slideLayout10.xml.rels"),
    asset!("ppt/printerSettings/printerSettings1.bin"),
];
const PKG_RELS: (&str, &[u8]) = asset!("_rels/.rels");
const THUMBNAIL: (&str, &[u8]) = asset!("docProps/thumbnail.jpeg");
/// A tiny transparent picture: the face of the audio object (PowerPoint hides it while the show runs)
const ICON_PNG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pptx/ppt/media/image1.png"));

const CT_SLIDE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const STATIC_OVERRIDES: &[(&str, &str)] = &[
    ("/docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
    ("/docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
    ("/ppt/presProps.xml", "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml"),
    ("/ppt/presentation.xml", "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"),
    ("/ppt/slideLayouts/slideLayout1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout10.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout11.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout2.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout3.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout4.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout5.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout6.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout7.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout8.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideLayouts/slideLayout9.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"),
    ("/ppt/slideMasters/slideMaster1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"),
    ("/ppt/tableStyles.xml", "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"),
    ("/ppt/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml"),
    ("/ppt/viewProps.xml", "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml"),
];

/// python-pptx's Inches(): EMU, truncated.
fn inches(v: f64) -> i64 {
    (v * 914400.0) as i64
}

/// Font size attribute: hundredths of a point of Pt(points) (python-pptx: int(points * 12700) // 127).
fn font_sz(points: f64) -> Result<i64, String> {
    let sz = ((points * 12700.0) as i64).div_euclid(127);
    if !(100..=400000).contains(&sz) {
        return Err(format!("font size {} out of range", sz));
    }
    Ok(sz)
}

/// XML text content the way python-pptx/lxml wrote it: control characters as _xHHHH_, then &, <, > escaped.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{00}'..='\u{08}' | '\u{0b}'..='\u{1f}' => out.push_str(&format!("_x{:04X}_", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn xml_attr(s: &str) -> String {
    xml_text(s).replace('"', "&quot;")
}

fn verse_lines(verse: &Value) -> Vec<String> {
    let text = verse.get("text").and_then(Value::as_str).unwrap_or("");
    py::strip(text).split('\n').map(py::strip).filter(|l| !l.is_empty()).map(String::from).collect()
}

fn verses_of(song: &Dict) -> Vec<Value> {
    match song.get("verses") {
        Some(Value::Array(v)) => v.clone(),
        _ => Vec::new(),
    }
}

/// Largest point size at which every block fits inside a text box of the given size (inches).
fn fit_pt(blocks: &[Vec<String>], box_w_in: f64, box_h_in: f64, line_spacing: f64, max_pt: f64) -> f64 {
    fit_font_size(
        blocks,
        (box_w_in - 2.0 * INSET_X) * 72.0,
        (box_h_in - 2.0 * INSET_Y) * 72.0,
        line_height_em() * line_spacing,
        max_pt,
    )
}

fn header_text(song: &Dict, verse: &Value, index: usize, total: usize) -> String {
    let song_title = py::get_str(song, "title", "");
    let verse_text = verse.get("text").and_then(Value::as_str).unwrap_or("");
    let progress = if detect_language(&format!("{}{}", verse_text, song_title)) == "he" {
        // A right-to-left mark after each number keeps PowerPoint from reordering the numbers and spaces
        format!("בית {}\u{200f} מתוך {}\u{200f}", index, total)
    } else {
        format!("Verse {} of {}", index, total)
    };
    format!("{}  •  {}", song_title, progress)
}

fn song_title(song: &Dict) -> &str {
    py::get_str(song, "title", "שיר ללא שם")
}

pub struct Sizes {
    pub title: f64,
    pub header: f64,
    pub verse: f64,
}

/// Point sizes used for every slide of a song: the largest that fit its longest title/header/verse.
pub fn song_font_sizes(song: &Dict) -> Sizes {
    let verses = verses_of(song);
    let headers: Vec<Vec<String>> =
        verses.iter().enumerate().map(|(i, v)| vec![header_text(song, v, i + 1, verses.len())]).collect();
    let blocks: Vec<Vec<String>> = verses.iter().map(verse_lines).collect();
    Sizes {
        title: fit_pt(&[vec![song_title(song).to_string()]], TITLE_BOX.2, TITLE_HEIGHT_IN, 1.0, MAX_TITLE_PT),
        header: fit_pt(&headers, HEADER_BOX.2, HEADER_BOX.3, 1.0, MAX_HEADER_PT),
        verse: MIN_VERSE_PT.max(fit_pt(&blocks, VERSE_BOX.2, VERSE_BOX.3, VERSE_LINE_SPACING, MAX_VERSE_PT)),
    }
}

fn hex(color: &str) -> String {
    color.trim_start_matches('#').to_uppercase()
}

/// One slide's XML, built shape by shape as python-pptx did.
struct SlideXml {
    shapes: String,
    next_id: u32,
    transition: Option<String>,
    timing: Option<String>,
    rels: Vec<(String, String)>, // (type, target); rId1 is the layout
}

impl SlideXml {
    fn new() -> SlideXml {
        SlideXml { shapes: String::new(), next_id: 2, transition: None, timing: None, rels: vec![(RT_LAYOUT.into(), "../slideLayouts/slideLayout7.xml".into())] }
    }

    fn rect(&mut self, x: i64, y: i64, cx: i64, cy: i64, color: &str) {
        let id = self.next_id;
        self.next_id += 1;
        self.shapes.push_str(&format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Rectangle {n}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr><p:style><a:lnRef idx="1"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="3"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="2"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></p:style><p:txBody><a:bodyPr rtlCol="0" anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/></a:p></p:txBody></p:sp>"#,
            id = id,
            n = id - 1,
            c = hex(color)
        ));
    }

    /// The decorative music notes around the edge, each a freeform shape (one path per part, so overlaps keep one color).
    fn music_notes(&mut self, theme: &Theme) {
        let color = hex(&notes::note_color(theme));
        let (sx, sy) = (SLIDE_WIDTH as f64 / 1280.0, SLIDE_HEIGHT as f64 / 720.0);
        for note in notes::frame_notes() {
            let (l, t, r, b) = notes::bounds(&note);
            let (x, y) = ((l * sx).floor() as i64, (t * sy).floor() as i64);
            let (cx, cy) = ((r * sx).ceil() as i64 - x, (b * sy).ceil() as i64 - y);
            let pt = |px: f64, py: f64| format!(r#"<a:pt x="{}" y="{}"/>"#, (px * sx).round() as i64 - x, (py * sy).round() as i64 - y);
            let mut paths = String::new();
            for part in &note {
                paths.push_str(&format!(r#"<a:path w="{}" h="{}">"#, cx, cy));
                for seg in part {
                    match *seg {
                        Seg::Move(px, py) => paths.push_str(&format!("<a:moveTo>{}</a:moveTo>", pt(px, py))),
                        Seg::Line(px, py) => paths.push_str(&format!("<a:lnTo>{}</a:lnTo>", pt(px, py))),
                        Seg::Cubic(x1, y1, x2, y2, px, py) => {
                            paths.push_str(&format!("<a:cubicBezTo>{}{}{}</a:cubicBezTo>", pt(x1, y1), pt(x2, y2), pt(px, py)))
                        }
                    }
                }
                paths.push_str("<a:close/></a:path>");
            }
            let id = self.next_id;
            self.next_id += 1;
            self.shapes.push_str(&format!(
                r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Music Note {n}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="l" t="t" r="r" b="b"/><a:pathLst>{paths}</a:pathLst></a:custGeom><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>"#,
                id = id,
                n = id - 1,
                x = x,
                y = y,
                cx = cx,
                cy = cy,
                paths = paths,
                c = color
            ));
        }
    }

    fn textbox(&mut self, b: (f64, f64, f64, f64), paragraphs: &[String]) {
        let id = self.next_id;
        self.next_id += 1;
        self.shapes.push_str(&format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="TextBox {n}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="square" anchor="ctr"><a:spAutoFit/></a:bodyPr><a:lstStyle/>{p}</p:txBody></p:sp>"#,
            id = id,
            n = id - 1,
            x = inches(b.0),
            y = inches(b.1),
            cx = inches(b.2),
            cy = inches(b.3),
            p = paragraphs.concat()
        ));
    }

    fn rel(&mut self, kind: &str, target: &str) -> String {
        self.rels.push((kind.into(), target.into()));
        format!("rId{}", self.rels.len())
    }

    fn xml(&self) -> String {
        format!(
            r#"{decl}<p:sld {ns}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>{tr}{tm}</p:sld>"#,
            decl = XML_DECL,
            ns = NS,
            shapes = self.shapes,
            tr = self.transition.as_deref().unwrap_or(""),
            tm = self.timing.as_deref().unwrap_or("")
        )
    }

    fn rels_xml(&self) -> String {
        let rels: String = self
            .rels
            .iter()
            .enumerate()
            .map(|(i, (t, target))| format!(r#"<Relationship Id="rId{}" Type="{}" Target="{}"/>"#, i + 1, t, target))
            .collect();
        format!(r#"{}<Relationships xmlns="{}">{}</Relationships>"#, XML_DECL, REL_NS, rels)
    }
}

struct Para<'a> {
    text: &'a str,
    size_pt: f64,
    bold: bool,
    color: &'a str,
    space_before_pt: Option<f64>,
    line_spacing: Option<f64>,
}

/// A paragraph: right-aligned right-to-left if it has Hebrew, otherwise centered.
fn paragraph(p: &Para) -> Result<String, String> {
    let align = if detect_language(p.text) == "he" { r#"algn="r" rtl="1""# } else { r#"algn="ctr""# };
    let mut ppr = String::new();
    if let Some(ls) = p.line_spacing {
        ppr.push_str(&format!(r#"<a:lnSpc><a:spcPct val="{}"/></a:lnSpc>"#, py::round_int(ls * 100000.0)));
    }
    if let Some(sb) = p.space_before_pt {
        ppr.push_str(&format!(r#"<a:spcBef><a:spcPts val="{}"/></a:spcBef>"#, font_sz(sb)?));
    }
    ppr.push_str(&format!(
        r#"<a:defRPr sz="{}"{}><a:solidFill><a:srgbClr val="{}"/></a:solidFill><a:latin typeface="Arial"/></a:defRPr>"#,
        font_sz(p.size_pt)?,
        if p.bold { r#" b="1""# } else { "" },
        hex(p.color)
    ));
    // Runs split at line breaks; empty runs are left out
    let mut runs = String::new();
    for (i, piece) in p.text.split(['\n', '\u{0b}']).enumerate() {
        if i > 0 {
            runs.push_str("<a:br/>");
        }
        if !piece.is_empty() {
            runs.push_str(&format!("<a:r><a:t>{}</a:t></a:r>", xml_text(piece)));
        }
    }
    Ok(format!("<a:p><a:pPr {}>{}</a:pPr>{}</a:p>", align, ppr, runs))
}

/// A stylish title slide introducing the song.
fn title_slide(song: &Dict, theme: &Theme, sizes: &Sizes, perf_title: &str) -> Result<SlideXml, String> {
    let mut s = SlideXml::new();
    s.rect(0, 0, SLIDE_WIDTH, SLIDE_HEIGHT, &theme.bg_color);
    s.music_notes(theme);
    s.rect(inches(1.5), inches(1.8), inches(10.333), inches(0.08), &theme.title_color);
    let mut paras = Vec::new();
    if !perf_title.is_empty() {
        paras.push(paragraph(&Para { text: perf_title, size_pt: 20.0, bold: false, color: &theme.text_color, space_before_pt: None, line_spacing: None })?);
    }
    paras.push(paragraph(&Para { text: song_title(song), size_pt: sizes.title, bold: true, color: &theme.title_color, space_before_pt: Some(14.0), line_spacing: None })?);
    let artist = py::get_str(song, "artist", "");
    if !artist.is_empty() {
        paras.push(paragraph(&Para { text: artist, size_pt: 32.0, bold: false, color: &theme.text_color, space_before_pt: Some(10.0), line_spacing: None })?);
    }
    s.textbox(TITLE_BOX, &paras);
    Ok(s)
}

/// A verse slide with the song header and progress indicator.
fn verse_slide(song: &Dict, verse: &Value, index: usize, total: usize, theme: &Theme, sizes: &Sizes) -> Result<SlideXml, String> {
    let mut s = SlideXml::new();
    s.rect(0, 0, SLIDE_WIDTH, SLIDE_HEIGHT, &theme.bg_color);
    s.music_notes(theme);
    let header = header_text(song, verse, index, total);
    let p = paragraph(&Para { text: &header, size_pt: sizes.header, bold: true, color: &theme.title_color, space_before_pt: None, line_spacing: None })?;
    s.textbox(HEADER_BOX, &[p]);
    s.rect(inches(1.0), inches(1.3), inches(11.333), inches(0.02), &theme.title_color);
    let mut paras = Vec::new();
    for line in verse_lines(verse) {
        paras.push(paragraph(&Para { text: &line, size_pt: sizes.verse, bold: true, color: &theme.text_color, space_before_pt: None, line_spacing: Some(VERSE_LINE_SPACING) })?);
    }
    if paras.is_empty() {
        // python-pptx's new text box starts with one empty paragraph
        paras.push("<a:p/>".into());
    }
    s.textbox(VERSE_BOX, &paras);
    Ok(s)
}

pub struct SlidePlan {
    pub title: f64,
    pub verses: Vec<f64>,
    pub audio_on_title: bool,
}

/// Seconds each of the song's slides is shown so the deck follows the audio, or None if the verses have no timings.
///
/// A verse slide lasts from its start to the next verse's start (so gaps stay on screen and the slides never drift
/// from the audio); the last one lasts until its end_time, else the end of the audio. The audio starts on the title
/// slide, which then lasts until the first verse starts; if that is (almost) at 0 the title slide is a short lead-in
/// and the audio starts on the first verse slide.
pub fn slide_durations(song: &Dict, audio_length: f64) -> Option<SlidePlan> {
    let verses = verses_of(song);
    if verses.is_empty() {
        return None;
    }
    let mut starts = Vec::new();
    for v in &verses {
        match v.get("start_time") {
            None | Some(Value::Null) => return None,
            Some(t) => starts.push(py::float(t)?),
        }
    }
    if starts.windows(2).any(|w| w[1] < w[0]) {
        return None;
    }
    let last_start = *starts.last().unwrap();
    let last_end = verses.last().unwrap().get("end_time").filter(|v| !v.is_null()).and_then(py::float);
    let end = match last_end {
        Some(e) if e > last_start => e,
        _ if audio_length > last_start => audio_length,
        _ => last_start + LAST_VERSE_FALLBACK_SECONDS,
    };
    let mut durations: Vec<f64> = starts.iter().zip(starts.iter().skip(1).chain(std::iter::once(&end))).map(|(a, b)| b - a).collect();
    if starts[0] >= MIN_TITLE_SECONDS {
        return Some(SlidePlan { title: starts[0], verses: durations, audio_on_title: true });
    }
    // The first verse slide carries the audio, which starts at 0 of the song, so it also covers the time before its start
    durations[0] += starts[0];
    Some(SlidePlan { title: LEAD_IN_SECONDS, verses: durations, audio_on_title: false })
}

fn auto_advance(seconds: f64) -> String {
    format!(r#"<p:transition advTm="{}"/>"#, 1.max(py::round_int(seconds * 1000.0)))
}

/// Timing tree: play from the start as soon as the slide appears; the media node spans `across_slides` slides.
fn audio_timing(shape_id: u32, across_slides: usize) -> String {
    format!(
        r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst><p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"><p:tn val="2"/></p:cond></p:stCondLst><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="5" presetID="1" presetClass="mediacall" presetSubtype="0" fill="hold" nodeType="afterEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:cmd type="call" cmd="playFrom(0.0)"><p:cBhvr><p:cTn id="6" dur="1" fill="hold"/><p:tgtEl><p:spTgt spid="{id}"/></p:tgtEl></p:cBhvr></p:cmd></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq><p:audio><p:cMediaNode vol="100000" numSld="{n}" showWhenStopped="0"><p:cTn id="7" fill="hold" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:endCondLst><p:cond evt="onStopAudio" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:endCondLst></p:cTn><p:tgtEl><p:spTgt spid="{id}"/></p:tgtEl></p:cMediaNode></p:audio></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#,
        id = shape_id,
        n = across_slides
    )
}

/// The media file parts of the package: partname -> bytes and content type
struct Media {
    name: String,
    content_type: Option<&'static str>,
    data: Vec<u8>,
}

/// Embed the audio in the slide, set to start automatically and keep playing across the next slides.
fn embed_audio(slide: &mut SlideXml, audio_name: &str, across_slides: usize) {
    let audio_rid = slide.rel(RT_AUDIO, &format!("../media/{}", audio_name));
    let media_rid = slide.rel(RT_MEDIA, &format!("../media/{}", audio_name));
    let image_rid = slide.rel(RT_IMAGE, "../media/image1.png");
    let id = slide.next_id;
    slide.next_id += 1;
    slide.shapes.push_str(&format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="Song audio" descr="image.png"><a:hlinkClick r:id="" action="ppaction://media"/></p:cNvPr><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr><a:audioFile r:link="{a}"/><p:extLst><p:ext uri="{{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}}"><p14:media xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" r:embed="{m}"/></p:ext></p:extLst></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed="{i}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="91440" y="91440"/><a:ext cx="274320" cy="274320"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        id = id,
        a = audio_rid,
        m = media_rid,
        i = image_rid
    ));
    slide.timing = Some(audio_timing(id, across_slides));
}

fn content_type_for(ext: &str) -> Option<&'static str> {
    AUDIO_CONTENT_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, ct)| *ct)
}

/// The audio as PowerPoint can play it: the file itself, or an MP3 copy. None if it can't be converted.
fn powerpoint_audio(audio_path: &Path) -> Option<(String, Vec<u8>)> {
    let ext = audio_path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
    if content_type_for(&ext).is_some() {
        return std::fs::read(audio_path).ok().map(|d| (ext, d));
    }
    match audio::convert_to_mp3(audio_path, 192) {
        Ok(data) => Some((".mp3".into(), data)),
        Err(e) => {
            crate::log::warn(&format!("Audio for the deck could not be converted to MP3: {}", e));
            None
        }
    }
}

fn audio_duration(song: &Dict, audio_path: &Path) -> f64 {
    let probed = audio::probe_duration(audio_path);
    if probed != 0.0 {
        return probed;
    }
    song.get("duration").and_then(py::float).filter(|d| *d != 0.0).unwrap_or(0.0)
}

/// Generate a 16:9 widescreen deck for a setlist or song; returns its path.
/// With `wait_for_click`, each song with audio stays on its title slide until the presenter clicks; the audio and the
/// timed slides then start from that click.
pub fn generate_presentation(songs: &[Dict], title: &str, filename_prefix: &str, theme: Option<&Dict>, wait_for_click: bool) -> Result<PathBuf, String> {
    let theme = resolve_theme(theme);
    let dir = &paths().presentations;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    let mut slides: Vec<SlideXml> = Vec::new();
    let mut media: Vec<Media> = Vec::new();
    let mut icon_used = false;

    for song in songs {
        let verses = verses_of(song);
        let sizes = song_font_sizes(song);
        let first = slides.len();
        slides.push(title_slide(song, &theme, &sizes, title)?);
        for (i, verse) in verses.iter().enumerate() {
            slides.push(verse_slide(song, verse, i + 1, verses.len(), &theme, &sizes)?);
        }
        let mut count = slides.len() - first;

        let Some(audio_path) = song.get("audio_path").and_then(Value::as_str).map(PathBuf::from).filter(|p| p.exists()) else {
            continue;
        };
        let Some((ext, data)) = powerpoint_audio(&audio_path) else { continue };
        let plan = slide_durations(song, audio_duration(song, &audio_path));
        // Seconds each slide of the song is shown (None: until the presenter clicks)
        let mut durations: Option<Vec<Option<f64>>> =
            plan.as_ref().map(|p| std::iter::once(p.title).chain(p.verses.iter().copied()).map(Some).collect());
        let mut audio_slide = if plan.as_ref().is_some_and(|p| !p.audio_on_title) && count > 1 { 1 } else { 0 };
        if wait_for_click {
            // The first title slide waits for a click; the audio starts on the slide after it
            if plan.as_ref().is_some_and(|p| p.audio_on_title) {
                // An identical copy of the title carries the audio and the intro time, so nothing changes on screen
                slides.insert(first + 1, title_slide(song, &theme, &sizes, title)?);
                count += 1;
                if let Some(d) = durations.as_mut() {
                    d.insert(0, None);
                }
                audio_slide = 1;
            } else if plan.is_some() {
                if let Some(d) = durations.as_mut() {
                    d[0] = None;
                }
            } else if count > 1 {
                audio_slide = 1;
            }
        }
        if let Some(durations) = &durations {
            for (slide, secs) in slides[first..].iter_mut().zip(durations) {
                if let Some(secs) = secs {
                    slide.transition = Some(auto_advance(*secs));
                }
            }
        }
        // Like python-pptx's next_partname: the first free /ppt/media/audioN<ext>
        let mut n = 1;
        let name = loop {
            let candidate = format!("audio{}{}", n, ext);
            if !media.iter().any(|m| m.name == candidate) {
                break candidate;
            }
            n += 1;
        };
        embed_audio(&mut slides[first + audio_slide], &name, count - audio_slide);
        media.push(Media { name, content_type: content_type_for(&ext), data });
        icon_used = true;
    }

    let timestamp = py::time_now() as i64;
    let out_path = dir.join(format!("{}_{}.pptx", py::clean_prefix(filename_prefix), timestamp));
    write_package(&out_path, &slides, &media, icon_used)?;
    Ok(out_path)
}

fn content_types_xml(slide_count: usize, media: &[Media], icon_used: bool) -> String {
    let mut defaults: Vec<(&str, &str)> = vec![
        ("bin", "application/vnd.openxmlformats-officedocument.presentationml.printerSettings"),
        ("jpeg", "image/jpeg"),
        ("rels", "application/vnd.openxmlformats-package.relationships+xml"),
        ("xml", "application/xml"),
    ];
    if icon_used {
        defaults.push(("png", "image/png"));
    }
    defaults.sort();
    let mut overrides: Vec<(String, String)> = STATIC_OVERRIDES.iter().map(|(p, c)| (p.to_string(), c.to_string())).collect();
    for i in 1..=slide_count {
        overrides.push((format!("/ppt/slides/slide{}.xml", i), CT_SLIDE.into()));
    }
    for m in media {
        overrides.push((format!("/ppt/media/{}", m.name), m.content_type.unwrap_or("application/octet-stream").into()));
    }
    overrides.sort();
    let mut xml = format!(r#"{}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#, XML_DECL);
    for (ext, ct) in defaults {
        xml.push_str(&format!(r#"<Default Extension="{}" ContentType="{}"/>"#, ext, ct));
    }
    for (part, ct) in overrides {
        xml.push_str(&format!(r#"<Override PartName="{}" ContentType="{}"/>"#, xml_attr(&part), ct));
    }
    xml.push_str("</Types>");
    xml
}

fn write_package(out_path: &Path, slides: &[SlideXml], media: &[Media], icon_used: bool) -> Result<(), String> {
    let file = std::fs::File::create(out_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, data: &[u8]| -> Result<(), String> {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())
    };

    // Same order python-pptx wrote the parts in
    put("[Content_Types].xml", content_types_xml(slides.len(), media, icon_used).as_bytes())?;
    put(PKG_RELS.0, PKG_RELS.1)?;
    for (name, data) in HEAD_PARTS {
        put(name, data)?;
    }
    let ids: String = (0..slides.len()).map(|i| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 256 + i, 7 + i)).collect();
    put("ppt/presentation.xml", PRESENTATION_XML.replace("{SLIDE_IDS}", &ids).as_bytes())?;
    let mut rels = String::from(concat!(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>"#,
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings" Target="printerSettings/printerSettings1.bin"/>"#,
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/presProps" Target="presProps.xml"/>"#,
        r#"<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/viewProps" Target="viewProps.xml"/>"#,
        r#"<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#,
        r#"<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles" Target="tableStyles.xml"/>"#
    ));
    for i in 0..slides.len() {
        rels.push_str(&format!(r#"<Relationship Id="rId{}" Type="{}" Target="slides/slide{}.xml"/>"#, 7 + i, RT_SLIDE, i + 1));
    }
    put("ppt/_rels/presentation.xml.rels", format!(r#"{}<Relationships xmlns="{}">{}</Relationships>"#, XML_DECL, REL_NS, rels).as_bytes())?;
    for (name, data) in TEMPLATE_PARTS {
        put(name, data)?;
    }
    let mut written: Vec<String> = Vec::new();
    for (i, slide) in slides.iter().enumerate() {
        put(&format!("ppt/slides/slide{}.xml", i + 1), slide.xml().as_bytes())?;
        put(&format!("ppt/slides/_rels/slide{}.xml.rels", i + 1), slide.rels_xml().as_bytes())?;
        // Media parts follow the first slide that refers to them
        for (_, target) in slide.rels.iter().skip(1) {
            let name = target.trim_start_matches("../media/").to_string();
            if written.contains(&name) {
                continue;
            }
            if name == "image1.png" {
                put("ppt/media/image1.png", ICON_PNG)?;
            } else if let Some(m) = media.iter().find(|m| m.name == name) {
                put(&format!("ppt/media/{}", m.name), &m.data)?;
            }
            written.push(name);
        }
    }
    put(THUMBNAIL.0, THUMBNAIL.1)?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}
