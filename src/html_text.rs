//! Plain text of a web page, exactly as the Python app made it (its HTMLParser subclass `_PageText` fed once, never
//! closed, plus `html.unescape` and the charset rules). The lyrics prompt contains this text, so it must match.
//!
//! The scanners below reproduce the regular expressions of CPython 3.14's html/parser.py (several use lookbehind,
//! which the regex crate lacks) and their backtracking behavior.

use regex::Regex;
use std::sync::OnceLock;

use crate::py_tables::{CODEC_ALIASES, HTML5_ENTITIES, SINGLE_BYTE_CODECS};

// ---------------------------------------------------------------- html.unescape

fn entity(name: &str) -> Option<&'static str> {
    HTML5_ENTITIES.binary_search_by(|(k, _)| (*k).cmp(name)).ok().map(|i| HTML5_ENTITIES[i].1)
}

fn invalid_charref(num: u32) -> Option<&'static str> {
    Some(match num {
        0x00 => "\u{fffd}",
        0x0d => "\r",
        0x80 => "\u{20ac}",
        0x81 => "\u{81}",
        0x82 => "\u{201a}",
        0x83 => "\u{0192}",
        0x84 => "\u{201e}",
        0x85 => "\u{2026}",
        0x86 => "\u{2020}",
        0x87 => "\u{2021}",
        0x88 => "\u{02c6}",
        0x89 => "\u{2030}",
        0x8a => "\u{0160}",
        0x8b => "\u{2039}",
        0x8c => "\u{0152}",
        0x8d => "\u{8d}",
        0x8e => "\u{017d}",
        0x8f => "\u{8f}",
        0x90 => "\u{90}",
        0x91 => "\u{2018}",
        0x92 => "\u{2019}",
        0x93 => "\u{201c}",
        0x94 => "\u{201d}",
        0x95 => "\u{2022}",
        0x96 => "\u{2013}",
        0x97 => "\u{2014}",
        0x98 => "\u{02dc}",
        0x99 => "\u{2122}",
        0x9a => "\u{0161}",
        0x9b => "\u{203a}",
        0x9c => "\u{0153}",
        0x9d => "\u{9d}",
        0x9e => "\u{017e}",
        0x9f => "\u{0178}",
        _ => return None,
    })
}

fn invalid_codepoint(num: u32) -> bool {
    matches!(num, 0x1..=0x8 | 0xb | 0xe..=0x1f | 0x7f..=0x9f | 0xfdd0..=0xfdef) || (num & 0xfffe == 0xfffe && num <= 0x10ffff)
}

fn replace_charref(s: &str) -> String {
    if let Some(digits) = s.strip_prefix('#') {
        let (radix, digits) = match digits.strip_prefix(['x', 'X']) {
            Some(hex) => (16, hex),
            None => (10, digits),
        };
        let digits = digits.trim_end_matches(';');
        // Python ints don't overflow; anything this long is far beyond U+10FFFF anyway
        let num = u64::from_str_radix(digits.trim_start_matches('0'), radix).ok().or(if digits.trim_start_matches('0').is_empty() { Some(0) } else { None });
        let Some(num) = num.filter(|n| *n <= 0x10FFFF + 1) else { return "\u{fffd}".into() };
        let num = num as u32;
        if let Some(r) = invalid_charref(num) {
            return r.into();
        }
        if (0xD800..=0xDFFF).contains(&num) || num > 0x10FFFF {
            return "\u{fffd}".into();
        }
        if invalid_codepoint(num) {
            return String::new();
        }
        return char::from_u32(num).map(String::from).unwrap_or_else(|| "\u{fffd}".into());
    }
    if let Some(v) = entity(s) {
        return v.into();
    }
    // The longest matching prefix (as the standard defines it), by code points
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for x in (2..chars.len()).rev() {
        let cut = chars[x].0;
        if let Some(v) = entity(&s[..cut]) {
            return format!("{}{}", v, &s[cut..]);
        }
    }
    format!("&{}", s)
}

/// Python's html.unescape.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    static CHARREF: OnceLock<Regex> = OnceLock::new();
    let re = CHARREF.get_or_init(|| Regex::new(r"&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\x0c <&#;]{1,32};?)").unwrap());
    re.replace_all(s, |c: &regex::Captures| replace_charref(&c[1])).into_owned()
}

// ---------------------------------------------------------------- the parser's scanners

/// Byte-index helpers over the page text (positions are byte offsets into a &str).
struct Text<'a> {
    s: &'a str,
    b: &'a [u8],
}

fn is_ws(c: u8) -> bool {
    matches!(c, b'\t' | b'\n' | b'\r' | b'\x0c' | b' ')
}

impl<'a> Text<'a> {
    fn at(&self, i: usize) -> Option<u8> {
        self.b.get(i).copied()
    }

    /// Character (not byte) length at i, for skipping over a "[^...]" class that matches any code point.
    fn char_len(&self, i: usize) -> usize {
        self.s[i..].chars().next().map_or(1, |c| c.len_utf8())
    }

    /// `[^<set>]*` from i: end position.
    fn skip_not(&self, mut i: usize, set: &[u8]) -> usize {
        while let Some(c) = self.at(i) {
            if set.contains(&c) {
                break;
            }
            i += self.char_len(i);
        }
        i
    }

    fn skip_ws(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(is_ws) {
            i += 1;
        }
        i
    }

    /// One attribute value after "=": '...' | "..." | (?!['"])[^>\t\n\r\f ]*  -> end, or None if no alternative fits
    fn attr_value(&self, i: usize) -> Option<usize> {
        match self.at(i) {
            Some(q @ (b'\'' | b'"')) => {
                let close = self.s[i + 1..].find(q as char)?;
                Some(i + 1 + close + 1)
            }
            _ => Some(self.skip_not(i, b">\t\n\r\x0c ")),
        }
    }

    /// `(?:[\t\n\r\f ]*=[\t\n\r\f ]*VALUE)?` from i: end (i itself when the group doesn't match).
    fn value_indicator(&self, i: usize) -> usize {
        let j = self.skip_ws(i);
        if self.at(j) != Some(b'=') {
            return i;
        }
        let k = self.skip_ws(j + 1);
        // Backtracking over the trailing whitespace can't help: VALUE alternatives would see the same character
        match self.attr_value(k) {
            Some(end) => end,
            None => {
                // '...' unclosed: try giving back whitespace before the quote (only whitespace, never matches a value)
                let mut back = k;
                while back > j + 1 {
                    back -= 1;
                    if let Some(end) = self.attr_value(back) {
                        return end;
                    }
                }
                i
            }
        }
    }

    /// Attribute name: (?<=['"\t\n\r\f /])[^\t\n\r\f />][^\t\n\r\f /=>]*  -> end, or None.
    fn attr_name(&self, i: usize) -> Option<usize> {
        if i == 0 || !matches!(self.at(i - 1), Some(b'\'' | b'"' | b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/')) {
            return None;
        }
        match self.at(i) {
            None | Some(b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/' | b'>') => None,
            Some(_) => Some(self.skip_not(i + self.char_len(i), b"\t\n\r\x0c /=>")),
        }
    }

    /// locatetagend.match(rawdata, i): i is at the tag name's first letter. Returns the match end.
    fn locate_tag_end(&self, i: usize) -> usize {
        let mut p = self.skip_not(i + 1, b"\t\n\r\x0c />");
        while self.at(p).is_some_and(|c| is_ws(c) || c == b'/') {
            p += 1;
        }
        loop {
            let Some(name_end) = self.attr_name(p) else { break };
            let mut q = self.value_indicator(name_end);
            while self.at(q).is_some_and(|c| is_ws(c) || c == b'/') {
                q += 1;
            }
            if q == p {
                break;
            }
            p = q;
        }
        if self.at(p) == Some(b'>') {
            p += 1;
        }
        p
    }

    /// tagfind_tolerant.match(rawdata, i): (tag name, end).
    fn tag_find(&self, i: usize) -> (&'a str, usize) {
        let name_end = self.skip_not(i + 1, b"\t\n\r\x0c />");
        let mut p = name_end;
        loop {
            match self.at(p) {
                Some(c) if is_ws(c) => p += 1,
                Some(b'/') if self.at(p + 1) != Some(b'>') => p += 1,
                _ => break,
            }
        }
        (&self.s[i..name_end], p)
    }

    /// attrfind_tolerant.match(rawdata, k): end, or None.
    fn attr_find(&self, k: usize) -> Option<usize> {
        let name_end = self.attr_name(k)?;
        let mut p = self.value_indicator(name_end);
        loop {
            match self.at(p) {
                Some(c) if is_ws(c) => p += 1,
                Some(b'/') if self.at(p + 1) != Some(b'>') => p += 1,
                _ => break,
            }
        }
        Some(p)
    }
}

// ---------------------------------------------------------------- _PageText

const HIDDEN: [&str; 6] = ["script", "style", "noscript", "svg", "head", "template"];
const BREAKS: [&str; 15] = ["br", "p", "div", "li", "tr", "td", "h1", "h2", "h3", "h4", "h5", "h6", "section", "article", "pre"];
const CDATA_CONTENT_ELEMENTS: [&str; 6] = ["script", "style", "xmp", "iframe", "noembed", "noframes"];
const RCDATA_CONTENT_ELEMENTS: [&str; 2] = ["textarea", "title"];

struct PageText {
    parts: Vec<String>,
    hidden: usize,
}

impl PageText {
    fn start(&mut self, tag: &str) {
        if HIDDEN.contains(&tag) {
            self.hidden += 1;
        } else if BREAKS.contains(&tag) {
            self.parts.push("\n".into());
        }
    }
    fn end(&mut self, tag: &str) {
        if HIDDEN.contains(&tag) {
            self.hidden = self.hidden.saturating_sub(1);
        } else if BREAKS.contains(&tag) {
            self.parts.push("\n".into());
        }
    }
    fn data(&mut self, d: String) {
        if self.hidden == 0 {
            self.parts.push(d);
        }
    }
}

/// HTMLParser(convert_charrefs=True).feed(html) driving _PageText (close() is never called, as in the Python app).
fn parse(html: &str) -> String {
    let t = Text { s: html, b: html.as_bytes() };
    let n = html.len();
    let mut out = PageText { parts: Vec::new(), hidden: 0 };
    // CDATA/RCDATA mode: the element, and whether its text is unescaped
    let mut cdata: Option<(String, bool)> = None;
    let mut i = 0usize;
    static TAIL_SEP: OnceLock<Regex> = OnceLock::new();
    let tail_sep = TAIL_SEP.get_or_init(|| Regex::new(r"[\t\n\r\x0c ;]").unwrap());

    while i < n {
        let j;
        match &cdata {
            None => match html[i..].find('<') {
                Some(off) => j = i + off,
                None => {
                    // A character reference may be cut in half at the end of the data: wait for more (forever).
                    // Python looks at the last 34 characters (code points, not bytes).
                    let last34 = html.char_indices().rev().nth(33).map_or(0, |(p, _)| p);
                    let from = i.max(last34);
                    if let Some(amp) = html[from..].rfind('&').map(|o| from + o) {
                        if !tail_sep.is_match(&html[amp..]) {
                            break;
                        }
                    }
                    j = n;
                }
            },
            Some((elem, _)) => {
                // </elem followed by whitespace, "/" or ">", case-insensitively (ASCII)
                let name = elem.as_bytes();
                let bytes = html.as_bytes();
                let mut found = None;
                let mut from = i;
                while let Some(off) = html[from..].find("</") {
                    let at = from + off;
                    let after = at + 2 + name.len();
                    if bytes.get(at + 2..after).is_some_and(|s| s.eq_ignore_ascii_case(name))
                        && matches!(bytes.get(after), Some(b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'/' | b'>'))
                    {
                        found = Some(at);
                        break;
                    }
                    from = at + 2;
                }
                match found {
                    Some(p) => j = p,
                    None => break,
                }
            }
        }
        if i < j {
            let escapable = cdata.as_ref().map_or(true, |c| c.1);
            let chunk = &html[i..j];
            out.data(if escapable { unescape(chunk) } else { chunk.to_string() });
        }
        i = j;
        if i == n {
            break;
        }
        // At '<'
        let next = t.at(i + 1);
        let k: Option<usize>;
        if next.is_some_and(|c| c.is_ascii_alphabetic()) {
            // Start tag
            let endpos = t.locate_tag_end(i + 1);
            if t.at(endpos.wrapping_sub(1)) != Some(b'>') || endpos == i + 1 {
                k = None;
            } else {
                let (name, mut kk) = t.tag_find(i + 1);
                let tag = name.to_lowercase();
                while kk < endpos {
                    match t.attr_find(kk) {
                        Some(e) => kk = e,
                        None => break,
                    }
                }
                let end = crate::py::strip(&html[kk..endpos]);
                if end != ">" && end != "/>" {
                    out.data(html[i..endpos].to_string());
                } else if end.ends_with("/>") {
                    out.start(&tag);
                    out.end(&tag);
                } else {
                    out.start(&tag);
                    if CDATA_CONTENT_ELEMENTS.contains(&tag.as_str()) || tag == "plaintext" {
                        cdata = Some((tag.clone(), false));
                    } else if RCDATA_CONTENT_ELEMENTS.contains(&tag.as_str()) {
                        cdata = Some((tag.clone(), true));
                    }
                }
                k = Some(endpos);
            }
        } else if html[i..].starts_with("</") {
            k = parse_endtag(&t, i, &mut out, &mut cdata);
        } else if html[i..].starts_with("<!--") {
            k = parse_comment(html, i);
        } else if html[i..].starts_with("<?") {
            k = html[i + 2..].find('>').map(|o| i + 2 + o + 1);
        } else if html[i..].starts_with("<!") {
            k = parse_declaration(html, i);
        } else if i + 1 < n {
            out.data("<".into());
            k = Some(i + 1);
        } else {
            break;
        }
        match k {
            Some(k) => i = k,
            None => break, // incomplete construct: Python waits for more data, which never comes
        }
    }
    out.parts.concat()
}

fn parse_endtag(t: &Text, i: usize, out: &mut PageText, cdata: &mut Option<(String, bool)>) -> Option<usize> {
    let html = t.s;
    html[i + 2..].find('>')?;
    if !t.at(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
        if t.at(i + 2) == Some(b'>') {
            return Some(i + 3);
        }
        // Bogus comment up to the next '>'
        return html[i + 2..].find('>').map(|o| i + 2 + o + 1);
    }
    let j = t.locate_tag_end(i + 2);
    if t.at(j.wrapping_sub(1)) != Some(b'>') {
        return None;
    }
    let (name, _) = t.tag_find(i + 2);
    out.end(&name.to_lowercase());
    *cdata = None;
    Some(j)
}

fn parse_comment(html: &str, i: usize) -> Option<usize> {
    static CLOSE: OnceLock<Regex> = OnceLock::new();
    let close = CLOSE.get_or_init(|| Regex::new(r"--!?>").unwrap());
    if let Some(m) = close.find(&html[i + 4..]) {
        return Some(i + 4 + m.end());
    }
    // Abrupt close: "-?>" right at the start
    let rest = &html[i + 4..];
    if rest.starts_with("->") {
        Some(i + 6)
    } else if rest.starts_with('>') {
        Some(i + 5)
    } else {
        None
    }
}

fn parse_declaration(html: &str, i: usize) -> Option<usize> {
    let rest = &html[i..];
    if rest.starts_with("<![CDATA[") {
        return rest[9..].find("]]>").map(|o| i + 9 + o + 3);
    }
    if rest.len() >= 9 && rest.as_bytes()[..9].eq_ignore_ascii_case(b"<!doctype") {
        return rest[9..].find('>').map(|o| i + 9 + o + 1);
    }
    rest[2..].find('>').map(|o| i + 2 + o + 1)
}

/// The visible text of an HTML page, with line breaks where the page has them (Python: html_to_text).
pub fn html_to_text(html: &str) -> String {
    static SPACES: OnceLock<Regex> = OnceLock::new();
    static AROUND_NL: OnceLock<Regex> = OnceLock::new();
    static MANY_NL: OnceLock<Regex> = OnceLock::new();
    let text = parse(html);
    let text = SPACES.get_or_init(|| Regex::new("[ \t\r\x0c\x0b\u{a0}]+").unwrap()).replace_all(&text, " ");
    let text = AROUND_NL.get_or_init(|| Regex::new(" *\n *").unwrap()).replace_all(&text, "\n");
    let text = MANY_NL.get_or_init(|| Regex::new("\n{3,}").unwrap()).replace_all(&text, "\n\n");
    crate::py::strip(&text).to_string()
}

// ---------------------------------------------------------------- decoding the page bytes

/// Python's codec name normalization and alias lookup; the canonical codec name, or None (LookupError).
fn lookup_codec(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    let mut norm = String::new();
    let mut pending = false;
    for c in lower.chars() {
        if c.is_alphanumeric() || c == '.' {
            if pending && !norm.is_empty() {
                norm.push('_');
            }
            pending = false;
            norm.push(c);
        } else {
            pending = true;
        }
    }
    for candidate in [norm.clone(), norm.replace('.', "_")] {
        if let Some((_, canon)) = CODEC_ALIASES.iter().find(|(a, _)| *a == candidate) {
            return Some(canon);
        }
    }
    None
}

/// bytes.decode(codec, errors="replace").
fn decode_with(codec: &str, data: &[u8]) -> String {
    if let Some((_, table)) = SINGLE_BYTE_CODECS.iter().find(|(n, _)| *n == codec) {
        return data.iter().map(|&b| char::from_u32(table[b as usize]).unwrap_or('\u{fffd}')).collect();
    }
    match codec {
        "utf_16" | "utf_16_le" | "utf_16_be" | "utf_32" => {
            let (body, big) = match (codec, data) {
                ("utf_16_be", d) => (d, true),
                ("utf_16_le", d) => (d, false),
                (_, [0xFE, 0xFF, rest @ ..]) => (rest, true),
                (_, [0xFF, 0xFE, rest @ ..]) => (rest, false),
                (_, d) => (d, false),
            };
            if codec == "utf_32" {
                return String::from_utf8_lossy(data).into_owned();
            }
            let units: Vec<u16> = body.chunks(2).map(|c| if c.len() == 2 { if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) } } else { 0xFFFD }).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(data).into_owned(),
    }
}

/// Older Hebrew sites declare windows-1255 only inside the page (<meta charset>), not in the HTTP header.
/// Order: the page's own declaration, the HTTP header's charset, then UTF-8.
pub fn decode_page(content: &[u8], header_charset: Option<&str>) -> String {
    static DECLARED: OnceLock<Regex> = OnceLock::new();
    let declared_re = DECLARED.get_or_init(|| Regex::new(r#"(?i)charset=["']?([A-Za-z0-9_-]+)"#).unwrap());
    let head: String = content[..content.len().min(4096)].iter().filter(|b| b.is_ascii()).map(|&b| b as char).collect();
    let declared = declared_re.captures(&head).map(|c| c[1].to_string());
    for name in [declared.as_deref(), header_charset, Some("utf-8")].into_iter().flatten() {
        if name.is_empty() {
            continue;
        }
        if let Some(codec) = lookup_codec(name) {
            return decode_with(codec, content);
        }
    }
    String::from_utf8_lossy(content).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_matches_python() {
        assert_eq!(unescape("a &amp; b &lt;&gt; &#x5d0;&#1488; &notit; &copy &#128; &#0;"), "a & b <> אא ¬it; © € \u{fffd}");
        assert_eq!(unescape("&nbsp;x&bogus;"), "\u{a0}x&bogus;");
    }

    #[test]
    fn text_matches_python() {
        // Expected strings printed by the Python app's html_to_text
        let page1 = "<html><head><meta http-equiv='Content-Type' content='text/html; charset=windows-1255'><title>x</title><style>.a{}</style></head><body><div>שיר&nbsp;בדיקה</div><p>שורה ראשונה<br>שורה שניה</p><script>var x=1;</script><ul><li>a</li><li>b &amp; c</li></ul>\n\n\n\n<td> x </td></body></html>";
        assert_eq!(html_to_text(page1), "שיר בדיקה\n\nשורה ראשונה\nשורה שניה\n\na\n\nb & c\n\nx");
        let page2 = "<html><body><h1>Title</h1><p>Line one<br/>Line two</p><pre>  keep   spaces </pre></body></html>";
        assert_eq!(html_to_text(page2), "Title\n\nLine one\n\nLine two\n\nkeep spaces");
    }

    #[test]
    fn charset_lookup() {
        assert_eq!(lookup_codec("windows-1255"), Some("cp1255"));
        assert_eq!(lookup_codec("UTF-8"), Some("utf_8"));
        assert_eq!(lookup_codec("iso-8859-8"), Some("iso8859_8"));
        assert_eq!(lookup_codec("x-unknown"), None);
    }
}
