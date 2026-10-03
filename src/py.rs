//! Small re-implementations of Python behaviors the app relied on, so stored files, file names and
//! prompts come out exactly as the Python version made them.

use serde_json::{Map, Value};
use unicode_general_category::{get_general_category, GeneralCategory as Gc};

pub type Dict = Map<String, Value>;

/// Python truthiness of a JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map_or(true, |f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Python `float(value)` for the values a song can hold (numbers, bools, numeric strings); None if it would raise.
pub fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => {
            let t = s.trim_matches(is_space).replace('_', "");
            let lower = t.to_ascii_lowercase();
            match lower.trim_start_matches(['+', '-']) {
                "inf" | "infinity" => Some(if lower.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY }),
                "nan" => Some(f64::NAN),
                _ => t.parse::<f64>().ok(),
            }
        }
        _ => None,
    }
}

/// `d.get(key)` as a string, or `default` when missing (Python code used `.get(key, "")`).
pub fn get_str<'a>(d: &'a Dict, key: &str, default: &'a str) -> &'a str {
    match d.get(key) {
        Some(Value::String(s)) => s,
        _ => default,
    }
}

/// Python `round(x, ndigits)`: correctly rounded, ties to even (Rust's fixed-precision formatting rounds the same way).
pub fn round_to(x: f64, ndigits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{:.*}", ndigits, x).parse().unwrap_or(x)
}

/// Python `round(x)` to an integer.
pub fn round_int(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Python `str.isspace()` for one character.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `str.isalnum()` for one character (letters and numbers of any script).
pub fn is_alnum(c: char) -> bool {
    matches!(
        get_general_category(c),
        Gc::UppercaseLetter
            | Gc::LowercaseLetter
            | Gc::TitlecaseLetter
            | Gc::ModifierLetter
            | Gc::OtherLetter
            | Gc::DecimalNumber
            | Gc::LetterNumber
            | Gc::OtherNumber
    )
}

/// Python regex `\w` for one character.
pub fn is_word(c: char) -> bool {
    c == '_' || is_alnum(c)
}

/// Python `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Python `str.lower()`.
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Python `urllib.parse.quote(s)` (safe="/"): UTF-8 percent-encoding of everything but unreserved characters and "/".
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Python `"".join(c if c.isalnum() else "_" for c in s)[:30]`: the file-name prefix of exports.
pub fn clean_prefix(s: &str) -> String {
    s.chars().map(|c| if is_alnum(c) { c } else { '_' }).take(30).collect()
}

/// Seconds since the epoch, like `time.time()`.
pub fn time_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------- JSON, written the way Python's json module does

/// Python `repr(float)`.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Shortest round-trip digits, as "d.ddde<exp>"
    let sci = format!("{:e}", x);
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let sign = if negative { "-" } else { "" };
    if (-4..16).contains(&exp) {
        // Fixed notation
        let point = exp + 1; // digits before the decimal point
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{}{}", sign, s)
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits };
        format!("{}{}e{}{:02}", sign, m, if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

fn write_json_str(out: &mut String, s: &str, ensure_ascii: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if ensure_ascii && (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_number(out: &mut String, n: &serde_json::Number) {
    if n.is_f64() {
        out.push_str(&float_repr(n.as_f64().unwrap()));
    } else {
        out.push_str(&n.to_string());
    }
}

fn write_json(out: &mut String, v: &Value, indent: Option<usize>, level: usize, ensure_ascii: bool) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_json_str(out, s, ensure_ascii),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                    if indent.is_none() {
                        out.push(' ');
                    }
                }
                if let Some(step) = indent {
                    out.push('\n');
                    out.push_str(&" ".repeat(step * (level + 1)));
                }
                write_json(out, item, indent, level + 1, ensure_ascii);
            }
            if let Some(step) = indent {
                out.push('\n');
                out.push_str(&" ".repeat(step * level));
            }
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                    if indent.is_none() {
                        out.push(' ');
                    }
                }
                if let Some(step) = indent {
                    out.push('\n');
                    out.push_str(&" ".repeat(step * (level + 1)));
                }
                write_json_str(out, k, ensure_ascii);
                out.push_str(": ");
                write_json(out, item, indent, level + 1, ensure_ascii);
            }
            if let Some(step) = indent {
                out.push('\n');
                out.push_str(&" ".repeat(step * level));
            }
            out.push('}');
        }
    }
}

/// `json.dumps(v, ensure_ascii=False, indent=2)`: how song and performance files are stored.
pub fn json_pretty(v: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, v, Some(2), 0, false);
    out
}

/// `json.dumps(v)` with its defaults (ASCII only, ", " and ": " separators): how the Gemini SDK sent requests.
pub fn json_dumps(v: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, v, None, 0, true);
    out
}

/// Text written the way Python's text mode writes it: "\n" becomes "\r\n" on Windows.
pub fn text_file_bytes(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repr_matches_python() {
        assert_eq!(float_repr(22.0), "22.0");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(1.5e-5), "1.5e-05");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(1791022240.1486337), "1791022240.1486337");
        assert_eq!(float_repr(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(float_repr(123456789012345680.0), "1.2345678901234568e+17");
        assert_eq!(float_repr(-2.5), "-2.5");
    }

    #[test]
    fn json_matches_python() {
        let v = json!({"a": [1, 2.0, "ש\n"], "b": {}, "c": [], "d": null});
        assert_eq!(json_pretty(&v), "{\n  \"a\": [\n    1,\n    2.0,\n    \"ש\\n\"\n  ],\n  \"b\": {},\n  \"c\": [],\n  \"d\": null\n}");
        assert_eq!(json_dumps(&v), "{\"a\": [1, 2.0, \"\\u05e9\\n\"], \"b\": {}, \"c\": [], \"d\": null}");
    }

    #[test]
    fn rounding_matches_python() {
        assert_eq!(round_to(2.675, 2), 2.67);
        assert_eq!(round_to(0.25, 1), 0.2);
        assert_eq!(round_to(22.05, 1), 22.1);
        assert_eq!(round_int(2.5), 2);
        assert_eq!(round_int(3.5), 4);
    }

    #[test]
    fn quote_matches_python() {
        assert_eq!(quote("שיר a/b?"), "%D7%A9%D7%99%D7%A8%20a/b%3F");
        assert_eq!(clean_prefix("עוד לא-תמו!"), "עוד_לא_תמו_");
    }
}
