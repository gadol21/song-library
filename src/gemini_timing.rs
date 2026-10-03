//! AI verse timing: Gemini listens to a recording and reports when each verse is sung.
//!
//! Two variants share `run_gemini` (upload, ask, parse, cost, delete uploads): timing against the song's own
//! YouTube video, or timing the song's audio (the karaoke version) guided by a YouTube version with a singer.
//! Prompts, schemas and request contents are the Python app's, character for character.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::gemini::{self, Client, GeminiError, Part, UploadedFile};
use crate::py;
use crate::youtube;

/// Start times closer than this to the previous verse's start are treated as inconsistent
const MIN_VERSE_GAP_SECONDS: f64 = 0.1;

fn response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verses": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "verse": {"type": "integer", "description": "The verse number, as given in the lyrics"},
                        "start": {"type": "string", "description": "Timestamp in the video (MM:SS.s, e.g. 01:13.4) when the first word is sung"},
                        "end": {"type": "string", "description": "Timestamp in the video (MM:SS.s, e.g. 01:19.0) when the last word ends"},
                    },
                    "required": ["verse", "start", "end"],
                },
            },
        },
        "required": ["verses"],
    })
}

fn singer_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verses": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "verse": {"type": "integer", "description": "The verse number, as given in the lyrics"},
                        "singer_start": {"type": "string", "description": "Timestamp in the SINGER recording (MM:SS.s) where the verse's first word is sung"},
                        "start": {"type": "string", "description": "Timestamp in the KARAOKE recording (MM:SS.s) where the verse starts"},
                        "end": {"type": "string", "description": "Timestamp in the KARAOKE recording (MM:SS.s) where the verse ends"},
                    },
                    "required": ["verse", "singer_start", "start", "end"],
                },
            },
        },
        "required": ["verses"],
    })
}

/// Seconds from a timestamp like "1:13.4", "01:13", "1:02:03" (also a plain number of seconds); None if unusable.
pub fn parse_timestamp(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(_) => None,
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let re = regex::Regex::new(r"^\s*(?:(\d+):)?(\d+):(\d{1,2}(?:[.,]\d+)?)\s*$").unwrap();
            if let Some(c) = re.captures(s) {
                let seconds: f64 = c[3].replace(',', ".").parse().ok()?;
                if seconds >= 60.0 {
                    return None;
                }
                let hours: f64 = c.get(1).map_or(Some(0.0), |h| h.as_str().parse().ok())?;
                let minutes: f64 = c[2].parse().ok()?;
                return Some(hours * 3600.0 + minutes * 60.0 + seconds);
            }
            py::float(&Value::String(py::strip(s).replace(',', ".")))
        }
        _ => None,
    }
}

/// Seconds as M:SS (H:MM:SS for long videos), rounded to the nearest second.
pub fn format_timestamp(seconds: f64) -> String {
    let total = py::round_int(seconds);
    let (hours, rest) = (total.div_euclid(3600), total.rem_euclid(3600));
    let (minutes, secs) = (rest.div_euclid(60), rest.rem_euclid(60));
    if hours != 0 {
        format!("{}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{}:{:02}", minutes, secs)
    }
}

fn numbered(verses: &[String]) -> String {
    verses.iter().enumerate().map(|(i, t)| format!("[{}]\n{}", i + 1, py::strip(t))).collect::<Vec<_>>().join("\n\n")
}

fn about(title: &str, artist: &str) -> String {
    [py::strip(title), py::strip(artist)].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" - ")
}

pub fn build_prompt(verses: &[String], title: &str, artist: &str, duration: f64) -> String {
    let about = about(title, artist);
    let mut s = String::from(
        "You are given a video of a song and the song's lyrics, split into numbered verses.\n\
         Listen to the audio and, for every verse, report when it is sung: the time its first word \
         starts and the time its last word ends.\n\n\
         Rules:\n\
         - Give every time as a timestamp from the very start of the video in MM:SS.s format, with one \
         digit after the seconds (for example 01:13.4 means 1 minute and 13.4 seconds).\n",
    );
    if duration > 0.0 {
        s.push_str(&format!("- The video is {} long, so no timestamp may be later than that.\n", format_timestamp(duration)));
    }
    s.push_str(
        "- The verses are listed in the order they are sung. Start times must increase from one verse \
         to the next, and a verse cannot start before the previous one ends.\n\
         - If the same words are sung more than once in the song, each numbered verse below is one \
         occurrence, in order. Match each verse to its own occurrence.\n\
         - Ignore intros, instrumental breaks, spoken parts, and scenes that are not part of the song.\n\
         - The singer may differ slightly from the written words; match by what is sung.\n\
         - Return an entry for every verse number, even if you are unsure.\n\n",
    );
    if !about.is_empty() {
        s.push_str(&format!("Song: {}\n\n", about));
    }
    s.push_str("Lyrics:\n\n");
    s.push_str(&numbered(verses));
    s
}

pub fn build_singer_prompt(verses: &[String], title: &str, artist: &str, karaoke_seconds: f64, singer_seconds: f64) -> String {
    let about = about(title, artist);
    let karaoke_length = if karaoke_seconds > 0.0 { format!(" It is {} long.", format_timestamp(karaoke_seconds)) } else { String::new() };
    let singer_length = if singer_seconds > 0.0 { format!(" It is {} long.", format_timestamp(singer_seconds)) } else { String::new() };
    let mut s = format!(
        "You are given two audio recordings of the same song, and the song's lyrics split into numbered verses.\n\
         - Recording 1 is the KARAOKE version: the backing track without a lead singer.{}\n\
         - Recording 2 is a version WITH A SINGER who sings the lyrics.{}\n\n",
        karaoke_length, singer_length
    );
    s.push_str(
        "Your task is to time every verse in the KARAOKE recording: the moment in the karaoke recording \
         when the verse should start being sung, and the moment it ends. Use the singer version to hear \
         when each verse is sung, then map those moments onto the karaoke recording.\n\n\
         Rules:\n\
         - The two recordings can differ in the length of the intro and outro, tempo, key, arrangement, \
         or even structure (for example a skipped or extra repeat). Do not assume that the same timestamp \
         means the same moment in both. Align them by the music: match sections such as intro, verses, \
         choruses, bridges and instrumental breaks.\n\
         - Give every time as a timestamp from the very start of that recording in MM:SS.s format, with \
         one digit after the seconds (for example 01:13.4 means 1 minute and 13.4 seconds). \
         \"singer_start\" is a time in the singer recording; \"start\" and \"end\" are times in the \
         karaoke recording and must not be later than its length.\n\
         - The verses are listed in the order they are sung. Start times must increase from one verse to \
         the next, and a verse cannot start before the previous one ends.\n\
         - If the same words are sung more than once in the song, each numbered verse below is one \
         occurrence, in order. Match each verse to its own occurrence.\n\
         - Ignore intros, instrumental breaks and spoken parts; do not assign them to a verse.\n\
         - Only the verses listed below matter. The singer version may sing extra verses that are not \
         listed (skip those), and the karaoke recording may leave out one of the listed verses (for \
         example playing an instrumental break instead). Use an empty string for \"start\" and \"end\" \
         only if a listed verse is really not played in the karaoke recording.\n\
         - Return an entry for every verse number, even if you are unsure.\n\n",
    );
    if !about.is_empty() {
        s.push_str(&format!("Song: {}\n\n", about));
    }
    s.push_str("Lyrics:\n\n");
    s.push_str(&numbered(verses));
    s
}

/// Python's int() of a verse number (int, float truncated, numeric string, bool); None if it would raise.
fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(*b as i64),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)),
        Value::String(s) => py::strip(s).replace('_', "").parse().ok(),
        _ => None,
    }
}

/// Turn the model's JSON into one {start_time, end_time} per verse (null when unusable), plus warnings.
///
/// Times that are missing, out of range or not increasing are dropped rather than guessed, so a bad answer
/// never produces overlapping verses.
pub fn parse_timings(raw: &str, verse_count: usize, duration: f64) -> Result<(Vec<(Option<f64>, Option<f64>)>, Vec<String>), GeminiError> {
    let entries = serde_json::from_str::<Value>(raw).ok().and_then(|v| v.get("verses").cloned());
    let Some(Value::Array(entries)) = entries else {
        return Err(GeminiError("התשובה של Gemini לא הייתה בפורמט צפוי. נסה שוב.".into()));
    };
    let mut by_number: std::collections::HashMap<i64, Value> = std::collections::HashMap::new();
    for entry in entries {
        if let Some(n) = entry.get("verse").and_then(py_int) {
            by_number.insert(n, entry);
        }
    }

    let mut timings: Vec<(Option<f64>, Option<f64>)> = Vec::new();
    let mut warnings = Vec::new();
    let mut previous_start: Option<f64> = None;
    for number in 1..=verse_count as i64 {
        let entry = by_number.get(&number);
        timings.push((None, None));
        let start = entry.and_then(|e| e.get("start")).and_then(parse_timestamp);
        let Some(start) = start else {
            let not_played = entry.is_some_and(|e| e.is_object() && e.get("start") == Some(&json!("")));
            warnings.push(format!("בית {}: {}", number, if not_played { "לא נמצא בהקלטה" } else { "לא התקבל תזמון" }));
            continue;
        };
        let start = py::round_to(start, 1);
        if start < 0.0 || (duration != 0.0 && start > duration + 1.0) {
            warnings.push(format!("בית {}: תזמון מחוץ לאורך הסרטון ({})", number, format_timestamp(start)));
            continue;
        }
        if previous_start.is_some_and(|p| start < p + MIN_VERSE_GAP_SECONDS) {
            warnings.push(format!("בית {}: תזמון לא עקבי עם הבית הקודם ({})", number, format_timestamp(start)));
            continue;
        }
        let last = timings.last_mut().unwrap();
        last.0 = Some(start);
        previous_start = Some(start);
        let end = entry.and_then(|e| e.get("end")).and_then(parse_timestamp);
        if let Some(end) = end {
            if py::round_to(end, 1) > start {
                last.1 = Some(py::round_to(end, 1));
            }
        }
    }

    // An end that runs into the next timed verse is dropped; the video then ends it when the next starts
    let timed: Vec<usize> = (0..timings.len()).filter(|&i| timings[i].0.is_some()).collect();
    for w in timed.windows(2) {
        let next_start = timings[w[1]].0.unwrap();
        if let Some(end) = timings[w[0]].1 {
            if end > next_start - MIN_VERSE_GAP_SECONDS {
                timings[w[0]].1 = None;
            }
        }
    }
    Ok((timings, warnings))
}

fn clean_verses(verses: &[String]) -> Result<Vec<String>, GeminiError> {
    let cleaned: Vec<String> = verses.iter().map(|v| py::strip(v).to_string()).filter(|v| !v.is_empty()).collect();
    if cleaned.is_empty() {
        return Err(GeminiError("אין בתים לתזמון. הוסף מילים קודם.".into()));
    }
    Ok(cleaned)
}

/// Upload files to Google, ask Gemini, and turn its answer into verse timings plus the cost.
/// `build` receives the uploaded files (same order as `files`) and returns the request parts.
/// Everything uploaded is deleted from Google afterwards.
async fn run_gemini(
    verses: &[String],
    files: &[PathBuf],
    build: impl Fn(&[UploadedFile]) -> Vec<Part<'_>>,
    schema: Value,
    duration: f64,
) -> Result<Map<String, Value>, GeminiError> {
    let key = gemini::api_key()?;
    let client = Client::new(key);
    let mut uploaded: Vec<UploadedFile> = Vec::new();
    let result = async {
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mime = gemini::guess_mime_type(&name).ok_or_else(|| {
                GeminiError("Unknown mime type: Could not determine the mimetype for your file\n    please set the `mime_type` argument".into())
            })?;
            let file = client.upload(path, mime).await?;
            uploaded.push(file);
            let ready = client.wait_until_active(uploaded.pop().unwrap()).await;
            match ready {
                Ok(f) => uploaded.push(f),
                Err(e) => return Err(e),
            }
        }
        let body = json!({
            "contents": gemini::user_contents(&build(&uploaded)),
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseJsonSchema": schema,
                "thinkingConfig": gemini::thinking_config(),
            },
        });
        let response = client.generate_content(&body).await?;
        let text = gemini::response_text(&response).filter(|t| !t.is_empty());
        let Some(text) = text else {
            return Err(GeminiError("Gemini לא החזיר תשובה (ייתכן שהתוכן נחסם). נסה שוב.".into()));
        };
        let (timings, warnings) = parse_timings(&text, verses.len(), duration)?;
        let mut out = Map::new();
        out.insert(
            "timings".into(),
            Value::Array(timings.iter().map(|(s, e)| json!({"start_time": s, "end_time": e})).collect()),
        );
        out.insert("warnings".into(), json!(warnings));
        out.insert("cost".into(), Value::Object(gemini::compute_cost(response.get("usageMetadata").unwrap_or(&Value::Null))));
        Ok(out)
    }
    .await;
    for f in &uploaded {
        client.delete_file(&f.name).await;
    }
    result
}

/// Time each verse of a song against its YouTube video: download a small copy, upload it, ask Gemini.
pub async fn time_verses_from_youtube(youtube_url: &str, verses: &[String], title: &str, artist: &str) -> Result<Map<String, Value>, GeminiError> {
    let verses = clean_verses(verses)?;
    gemini::api_key()?; // fail early, before downloading anything
    let work = tempfile::Builder::new().prefix("singalong-ai-").tempdir().map_err(|e| GeminiError(e.to_string()))?;
    let (video_path, info) = youtube::download_video(youtube_url, work.path()).await.map_err(|e| GeminiError(e.0))?;
    time_verses_in_video(&video_path, info.duration, &verses, title, artist).await
}

/// The Gemini part of time_verses_from_youtube, for an already downloaded video of `duration` seconds.
pub async fn time_verses_in_video(video_path: &Path, duration: f64, verses: &[String], title: &str, artist: &str) -> Result<Map<String, Value>, GeminiError> {
    let verses = clean_verses(verses)?;
    let prompt = build_prompt(&verses, title, artist, duration);
    let files = [video_path.to_path_buf()];
    let mut result = run_gemini(&verses, &files, |f| vec![Part::File(&f[0]), Part::Text(prompt.clone())], response_schema(), duration).await?;
    result.insert("video_seconds".into(), json!(duration));
    Ok(result)
}

/// Time the verses in a karaoke (backing track) recording, using a version with a singer as a guide.
pub async fn time_verses_from_singer_version(singer_url: &str, karaoke_audio: &Path, verses: &[String], title: &str, artist: &str) -> Result<Map<String, Value>, GeminiError> {
    let verses = clean_verses(verses)?;
    gemini::api_key()?;
    if !karaoke_audio.is_file() {
        return Err(GeminiError("לא נמצא קובץ השמע של השיר (גרסת הקריוקי).".into()));
    }
    let work = tempfile::Builder::new().prefix("singalong-ai-").tempdir().map_err(|e| GeminiError(e.to_string()))?;
    let (singer_bytes, singer_info) = youtube::download_audio_mp3(singer_url).await.map_err(|e| GeminiError(e.0))?;
    let singer_path = work.path().join("singer.mp3");
    std::fs::write(&singer_path, &singer_bytes).map_err(|e| GeminiError(e.to_string()))?;
    time_verses_with_singer_file(&singer_path, singer_info.duration, karaoke_audio, &verses, title, artist).await
}

/// The Gemini part of time_verses_from_singer_version, for an already downloaded singer.mp3 (its length per
/// YouTube, 0 if unknown).
pub async fn time_verses_with_singer_file(singer_path: &Path, singer_duration: f64, karaoke_audio: &Path, verses: &[String], title: &str, artist: &str) -> Result<Map<String, Value>, GeminiError> {
    let verses = clean_verses(verses)?;
    let singer_seconds = if singer_duration != 0.0 { singer_duration } else { crate::audio::probe_duration(singer_path) };
    let karaoke_seconds = crate::audio::probe_duration(karaoke_audio);
    let singer_path = singer_path.to_path_buf();

    let prompt = build_singer_prompt(&verses, title, artist, karaoke_seconds, singer_seconds);
    let files = [karaoke_audio.to_path_buf(), singer_path];
    let mut result = run_gemini(
        &verses,
        &files,
        |f| {
            vec![
                Part::Text("Recording 1 - the KARAOKE version (backing track without the lead singer):".into()),
                Part::File(&f[0]),
                Part::Text("Recording 2 - a version WITH A SINGER:".into()),
                Part::File(&f[1]),
                Part::Text(prompt.clone()),
            ]
        },
        singer_response_schema(),
        karaoke_seconds,
    )
    .await?;
    result.insert("karaoke_seconds".into(), json!(karaoke_seconds));
    result.insert("singer_seconds".into(), json!(singer_seconds));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp(&json!("01:13.4")), Some(73.4));
        assert_eq!(parse_timestamp(&json!("1:02:03")), Some(3723.0));
        assert_eq!(parse_timestamp(&json!("1:60")), None);
        assert_eq!(parse_timestamp(&json!(" 12,5 ")), Some(12.5));
        assert_eq!(parse_timestamp(&json!(true)), None);
        assert_eq!(format_timestamp(19.5), "0:20");
        assert_eq!(format_timestamp(20.5), "0:20");
        assert_eq!(format_timestamp(3725.0), "1:02:05");
    }

    #[test]
    fn parsing_matches_python() {
        let raw = r#"{"verses": [{"verse": 1, "start": "00:01.5", "end": "00:07.0"}, {"verse": 2, "start": "00:08.0", "end": "00:16.0"}, {"verse": 3, "start": "", "end": ""}]}"#;
        let (t, w) = parse_timings(raw, 3, 20.0).unwrap();
        assert_eq!(t, vec![(Some(1.5), Some(7.0)), (Some(8.0), Some(16.0)), (None, None)]);
        assert_eq!(w, vec!["בית 3: לא נמצא בהקלטה"]);
    }
}
