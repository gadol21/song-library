//! Songs and performances stored as files (the Python app's database.py).
//!
//! DATA_DIR/songs/<id>/ holds song.json, audio.* and a generated lyrics.lrc; DATA_DIR/performances/<id>.json
//! holds a setlist. Song and performance dicts are kept as JSON objects in their original key order, so any
//! field the page sends is stored as is.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::{ensure_dirs, paths};
use crate::log;
use crate::py::{self, Dict};

pub fn detect_language(text: &str) -> &'static str {
    if text.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c)) {
        "he"
    } else {
        "en"
    }
}

/// A filesystem-safe id from a title (Hebrew and English friendly).
pub fn slugify(text: &str) -> String {
    // Python: re.sub(r'[^\w֐-׿\s-]', '', text).strip(), then whitespace/underscore runs -> "-", lowercased
    let kept: String = text
        .chars()
        .filter(|&c| py::is_word(c) || ('\u{0590}'..='\u{05FF}').contains(&c) || py::is_space(c) || c == '-')
        .collect();
    let clean = py::strip(&kept);
    let mut slug = String::new();
    let mut in_run = false;
    for c in clean.chars() {
        if py::is_space(c) || c == '_' {
            if !in_run {
                slug.push('-');
            }
            in_run = true;
        } else {
            slug.push(c);
            in_run = false;
        }
    }
    let slug = py::lower(&slug);
    if slug.is_empty() {
        format!("song-{}", py::time_now() as i64)
    } else {
        slug
    }
}

/// Seconds as an LRC tag [mm:ss.xx].
pub fn format_lrc_time(seconds: f64) -> String {
    let mins = (seconds / 60.0).floor();
    let secs = seconds - 60.0 * mins; // Python's float % for non-negative values
    let secs = if secs < 0.0 { secs + 60.0 } else { secs };
    format!("[{:02}:{:05.2}]", mins as i64, secs)
}

/// Start time of a verse; untimed verses (missing or null) count as 0.
fn verse_start(verse: &Value) -> f64 {
    match verse.get("start_time") {
        None | Some(Value::Null) => 0.0,
        Some(v) => py::float(v).unwrap_or(0.0),
    }
}

/// End time of a verse; falls back to start + 5s when missing or null.
fn verse_end(verse: &Value) -> f64 {
    match verse.get("end_time") {
        None | Some(Value::Null) => verse_start(verse) + 5.0,
        Some(v) => py::float(v).unwrap_or_else(|| verse_start(verse) + 5.0),
    }
}

fn verses_of(d: &Dict) -> Vec<Value> {
    match d.get("verses") {
        Some(Value::Array(v)) => v.clone(),
        _ => Vec::new(),
    }
}

/// The verses as LRC text.
pub fn generate_lrc(verses: &[Value], title: &str, artist: &str) -> String {
    let mut lines = vec![format!("[ti:{}]", title), format!("[ar:{}]", artist), "[by:Sing-Along Studio]".into(), String::new()];
    let mut sorted: Vec<&Value> = verses.iter().collect();
    sorted.sort_by(|a, b| verse_start(a).partial_cmp(&verse_start(b)).unwrap_or(std::cmp::Ordering::Equal));
    for verse in sorted {
        let tag = format_lrc_time(verse_start(verse));
        let text = verse.get("text").and_then(Value::as_str).unwrap_or("");
        for (idx, line) in py::strip(text).split('\n').enumerate() {
            let line = py::strip(line);
            if !line.is_empty() {
                if idx == 0 {
                    lines.push(format!("{} {}", tag, line));
                } else {
                    lines.push(line.to_string());
                }
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

/// The song folder's audio.* files, in directory order (the first one is the song's audio).
fn audio_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_lowercase().starts_with("audio."))
        .map(|e| e.path())
        .collect()
}

fn read_json(path: &Path) -> Result<Dict, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    match serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())? {
        Value::Object(map) => Ok(map),
        _ => Err("not a JSON object".into()),
    }
}

fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, py::text_file_bytes(text))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn list_songs(search: Option<&str>, language: Option<&str>) -> Vec<Dict> {
    ensure_dirs();
    let mut songs = Vec::new();
    let Ok(entries) = std::fs::read_dir(&paths().songs) else { return songs };
    for entry in entries.flatten() {
        let folder = entry.path();
        let json_file = folder.join("song.json");
        if !folder.is_dir() || !json_file.exists() {
            continue;
        }
        let mut data = match read_json(&json_file) {
            Ok(d) => d,
            Err(e) => {
                log::error(&format!("Error reading {}: {}", json_file.display(), e));
                continue;
            }
        };
        let audio = audio_files(&folder);
        data.insert("has_audio".into(), json!(!audio.is_empty()));
        if let Some(first) = audio.first() {
            data.insert("audio_filename".into(), json!(file_name(first)));
        }
        if let Some(lang) = language.filter(|l| !l.is_empty() && *l != "all") {
            if data.get("language").and_then(Value::as_str) != Some(lang) {
                continue;
            }
        }
        if let Some(s) = search.filter(|s| !s.is_empty()) {
            let s = py::lower(s);
            let title = py::lower(py::get_str(&data, "title", ""));
            let artist = py::lower(py::get_str(&data, "artist", ""));
            if !title.contains(&s) && !artist.contains(&s) {
                continue;
            }
        }
        songs.push(data);
    }
    songs.sort_by_key(|s| py::lower(py::get_str(s, "title", "")));
    songs
}

pub fn get_song(song_id: &str) -> Option<Dict> {
    ensure_dirs();
    let folder = paths().songs.join(song_id);
    let json_file = folder.join("song.json");
    if !json_file.exists() {
        return None;
    }
    match read_json(&json_file) {
        Ok(mut data) => {
            let audio = audio_files(&folder);
            data.insert("has_audio".into(), json!(!audio.is_empty()));
            if let Some(first) = audio.first() {
                data.insert("audio_filename".into(), json!(file_name(first)));
                data.insert("audio_path".into(), json!(first.to_string_lossy()));
            }
            Some(data)
        }
        Err(e) => {
            log::error(&format!("Error loading song {}: {}", song_id, e));
            None
        }
    }
}

/// Save or update a song; returns the stored dict (with the audio fields when audio was given).
pub fn save_song(mut song: Dict, audio: Option<(&[u8], &str)>) -> std::io::Result<Dict> {
    ensure_dirs();
    let song_id = match song.get("id") {
        v if py::truthy(v) => match v.unwrap() {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        },
        _ => slugify(py::get_str(&song, "title", "untitled")),
    };
    song.insert("id".into(), json!(song_id));

    let verses = verses_of(&song);
    // Auto detect language if not explicitly provided
    if !py::truthy(song.get("language")) {
        let texts: Vec<&str> = verses.iter().map(|v| v.get("text").and_then(Value::as_str).unwrap_or("")).collect();
        let all_text = format!("{} {}", py::get_str(&song, "title", ""), texts.join(" "));
        song.insert("language".into(), json!(detect_language(&all_text)));
    }

    // Duration covers the verses (3 s after the last one ends)
    if !verses.is_empty() {
        let max_end = verses.iter().map(verse_end).fold(f64::NEG_INFINITY, f64::max);
        let duration = song.get("duration");
        let too_short = !py::truthy(duration) || duration.and_then(py::float).is_some_and(|d| d < max_end);
        if too_short {
            song.insert("duration".into(), json!(py::round_to(max_end + 3.0, 1)));
        }
    }

    let folder = paths().songs.join(&song_id);
    std::fs::create_dir_all(&folder)?;

    if let Some((bytes, ext)) = audio.filter(|(b, _)| !b.is_empty()) {
        for old in audio_files(&folder) {
            let _ = std::fs::remove_file(old);
        }
        let target = folder.join(format!("audio{}", ext));
        std::fs::write(&target, bytes)?;
        song.insert("has_audio".into(), json!(true));
        song.insert("audio_filename".into(), json!(file_name(&target)));
        song.insert("audio_path".into(), json!(target.to_string_lossy()));
    }

    song.insert("updated_at".into(), json!(py::time_now()));
    write_text(&folder.join("song.json"), &py::json_pretty(&Value::Object(song.clone())))?;
    let lrc = generate_lrc(&verses, py::get_str(&song, "title", ""), py::get_str(&song, "artist", ""));
    write_text(&folder.join("lyrics.lrc"), &lrc)?;
    Ok(song)
}

pub fn delete_song(song_id: &str) -> bool {
    let folder = paths().songs.join(song_id);
    folder.is_dir() && std::fs::remove_dir_all(&folder).is_ok()
}

pub fn lrc_path(song_id: &str) -> PathBuf {
    paths().songs.join(song_id).join("lyrics.lrc")
}

fn created_at(p: &Dict) -> f64 {
    p.get("created_at").and_then(py::float).unwrap_or(0.0)
}

pub fn list_performances() -> Vec<Dict> {
    ensure_dirs();
    let mut list = Vec::new();
    let Ok(entries) = std::fs::read_dir(&paths().performances) else { return list };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        match read_json(&path) {
            Ok(d) => list.push(d),
            Err(e) => log::error(&format!("Error loading performance {}: {}", path.display(), e)),
        }
    }
    // Newest first; a stable sort keeps the folder order for equal times, as Python's sort(reverse=True) does
    list.sort_by(|a, b| created_at(b).partial_cmp(&created_at(a)).unwrap_or(std::cmp::Ordering::Equal));
    list
}

/// A performance with its songs' full details ("songs") and their total length.
pub fn get_performance(perf_id: &str) -> Option<Dict> {
    ensure_dirs();
    let file = paths().performances.join(format!("{}.json", perf_id));
    if !file.exists() {
        return None;
    }
    let mut data = match read_json(&file) {
        Ok(d) => d,
        Err(e) => {
            log::error(&format!("Error loading performance {}: {}", perf_id, e));
            return None;
        }
    };
    let mut songs = Vec::new();
    let mut total = 0.0;
    if let Some(Value::Array(ids)) = data.get("song_ids") {
        for id in ids {
            let Some(id) = id.as_str() else { continue };
            if let Some(song) = get_song(id) {
                total += song.get("duration").and_then(py::float).unwrap_or(0.0);
                songs.push(Value::Object(song));
            }
        }
    }
    data.insert("songs".into(), Value::Array(songs));
    data.insert("total_duration".into(), json!(total));
    Some(data)
}

pub fn save_performance(mut perf: Dict) -> std::io::Result<Dict> {
    ensure_dirs();
    let perf_id = match perf.get("id") {
        v if py::truthy(v) => match v.unwrap() {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        },
        _ => slugify(py::get_str(&perf, "title", "performance")),
    };
    perf.insert("id".into(), json!(perf_id));
    if !py::truthy(perf.get("created_at")) {
        perf.insert("created_at".into(), json!(py::time_now()));
    }
    perf.insert("updated_at".into(), json!(py::time_now()));
    write_text(&paths().performances.join(format!("{}.json", perf_id)), &py::json_pretty(&Value::Object(perf.clone())))?;
    Ok(get_performance(&perf_id).unwrap_or(perf))
}

pub fn delete_performance(perf_id: &str) -> bool {
    let file = paths().performances.join(format!("{}.json", perf_id));
    file.exists() && std::fs::remove_file(&file).is_ok()
}

/// The songs of a performance dict (as returned by get_performance).
pub fn performance_songs(perf: &Dict) -> Vec<Dict> {
    match perf.get("songs") {
        Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_object().cloned()).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_match_python() {
        assert_eq!(slugify("עוד לא תמו כל פלאייך"), "עוד-לא-תמו-כל-פלאייך");
        assert_eq!(slugify("Stand By Me!"), "stand-by-me");
        assert_eq!(slugify("  Hello__World  x "), "hello-world-x");
        assert_eq!(slugify("A - B"), "a---b");
    }

    #[test]
    fn lrc_times_match_python() {
        assert_eq!(format_lrc_time(1.5), "[00:01.50]");
        assert_eq!(format_lrc_time(75.125), "[01:15.12]");
        assert_eq!(format_lrc_time(0.0), "[00:00.00]");
    }
}
