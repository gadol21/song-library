//! Gemini API over REST, sending exactly what the Python app's google-genai SDK sent: the same JSON bodies
//! (serialized like Python's json.dumps), the same resumable upload protocol and mime types.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::py;
use crate::py_tables::{MIME_ENCODINGS_MAP, MIME_SUFFIX_MAP, MIME_TYPES};

pub const GEMINI_MODEL: &str = "gemini-3.8-flash";

/// Reasoning effort: "low", "medium" or "high" (override with GEMINI_THINKING_LEVEL in .env).
/// Measured on two songs (15 runs): timing precision was the same at every level (within ~0.5 s) while
/// time and cost grew a lot: low ~5 s / $0.01, medium ~25 s / $0.03, high 1-3 min / up to $0.20.
/// So low is the default. If a song with many repeated choruses or a very different arrangement gets
/// confused, try GEMINI_THINKING_LEVEL=medium; high isn't worth it.
const THINKING_LEVEL_DEFAULT: &str = "low";
const THINKING_LEVELS: [&str; 3] = ["low", "medium", "high"];

pub fn thinking_level() -> String {
    let level = std::env::var("GEMINI_THINKING_LEVEL").unwrap_or_default().trim().to_lowercase();
    if THINKING_LEVELS.contains(&level.as_str()) {
        level
    } else {
        THINKING_LEVEL_DEFAULT.into()
    }
}

/// The SDK sent the level as its enum value
pub fn thinking_config() -> Value {
    json!({"thinking_level": thinking_level().to_uppercase()})
}

/// Paid-tier price in USD per 1M tokens: (valid through this date, input, output). Output includes
/// thinking tokens. Source: https://ai.google.dev/gemini-api/docs/pricing (introductory price until the end of 2026).
const PRICE_SCHEDULE: [(Option<(i32, u32, u32)>, f64, f64); 2] = [(Some((2026, 12, 31)), 0.75, 3.75), (None, 1.50, 7.50)];

/// Env var holding the key (first one set wins)
pub const API_KEY_VARS: [&str; 3] = ["GOOGLE_AI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"];

/// Google sometimes answers "high demand" (503); retry a few times before giving up
const BUSY_RETRIES: u32 = 3;
const BUSY_RETRY_DELAY_SECONDS: u64 = 8;

/// How long to wait for Google to finish processing the uploaded video
const FILE_PROCESSING_TIMEOUT_SECONDS: u64 = 180;

/// The SDK uploaded files in 8 MB chunks
const UPLOAD_CHUNK: usize = 8 * 1024 * 1024;

/// A user-presentable failure (the Python app's GeminiTimingError).
#[derive(Debug)]
pub struct GeminiError(pub String);

impl std::fmt::Display for GeminiError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An HTTP error answer from the API (the SDK's APIError).
#[derive(Debug)]
pub struct ApiError {
    pub code: u16,
    pub message: String,
}

// ---------------------------------------------------------------- cost

fn today() -> (i32, u32, u32) {
    use chrono::Datelike;
    let d = chrono::Local::now().date_naive();
    (d.year(), d.month(), d.day())
}

/// (price valid through, input $/1M tokens, output $/1M tokens) in effect on a given day.
fn price_on(day: (i32, u32, u32)) -> (Option<(i32, u32, u32)>, f64, f64) {
    for (through, input, output) in PRICE_SCHEDULE {
        match through {
            Some(t) if day <= t => return (Some(t), input, output),
            None => return (None, input, output),
            _ => {}
        }
    }
    unreachable!()
}

fn int_field(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// Cost of one request from Gemini's usage metadata (same keys, order and arithmetic as the Python dict).
pub fn compute_cost(usage: &Value) -> Map<String, Value> {
    let (through, input_price, output_price) = price_on(today());
    let input_tokens = int_field(usage, "promptTokenCount");
    let answer_tokens = int_field(usage, "candidatesTokenCount");
    let thinking_tokens = int_field(usage, "thoughtsTokenCount");
    // Thinking is billed as output. Trust Gemini's own total when it reports more than the parts add up to.
    let total_tokens = int_field(usage, "totalTokenCount");
    let output_tokens = (answer_tokens + thinking_tokens).max(total_tokens - input_tokens);

    let mut by_modality = Map::new();
    if let Some(Value::Array(details)) = usage.get("promptTokensDetails") {
        for d in details {
            let name = d.get("modality").and_then(Value::as_str).unwrap_or("None").to_lowercase();
            let count = int_field(d, "tokenCount");
            let prev = by_modality.get(&name).and_then(Value::as_i64).unwrap_or(0);
            by_modality.insert(name, json!(prev + count));
        }
    }
    let input_cost = input_tokens as f64 / 1_000_000.0 * input_price;
    let output_cost = output_tokens as f64 / 1_000_000.0 * output_price;
    let mut cost = Map::new();
    cost.insert("model".into(), json!(GEMINI_MODEL));
    cost.insert("input_tokens".into(), json!(input_tokens));
    cost.insert("input_tokens_by_modality".into(), Value::Object(by_modality));
    cost.insert("output_tokens".into(), json!(output_tokens - thinking_tokens));
    cost.insert("thinking_tokens".into(), json!(thinking_tokens));
    cost.insert("input_price_per_million".into(), json!(input_price));
    cost.insert("output_price_per_million".into(), json!(output_price));
    cost.insert("price_valid_through".into(), through.map_or(Value::Null, |(y, m, d)| json!(format!("{:04}-{:02}-{:02}", y, m, d))));
    cost.insert("input_cost_usd".into(), json!(input_cost));
    cost.insert("output_cost_usd".into(), json!(output_cost));
    cost.insert("total_cost_usd".into(), json!(input_cost + output_cost));
    cost
}

// ---------------------------------------------------------------- errors

pub fn api_key() -> Result<String, GeminiError> {
    for name in API_KEY_VARS {
        let value = std::env::var(name).unwrap_or_default();
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_string());
        }
    }
    Err(GeminiError(format!("לא נמצא מפתח API של Gemini. הוסף את {} לקובץ .env והפעל מחדש את האפליקציה.", API_KEY_VARS[0])))
}

/// "Please retry in 12h7m28.7s" from Google's quota error, as Hebrew text ("12 שעות ו-7 דקות"); "" if absent.
fn retry_hint(detail: &str) -> String {
    let re = regex::Regex::new(r"retry in (?:(\d+)h)?(?:(\d+)m)?(?:[\d.]+s)?").unwrap();
    let Some(c) = re.captures(detail) else { return String::new() };
    if c.get(1).is_none() && c.get(2).is_none() {
        return String::new();
    }
    let hours: u64 = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    let minutes: u64 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{} שעות", hours));
    }
    if minutes > 0 {
        parts.push(format!("{} דקות", minutes));
    }
    parts.join(" ו-")
}

/// Describe a Gemini API failure in Hebrew, without ever echoing the API key.
pub fn friendly_api_error(error: &ApiError, key: &str) -> GeminiError {
    let code = error.code;
    let detail = error.message.replace(key, "***");
    if matches!(code, 400 | 401 | 403) && detail.to_lowercase().contains("api key") {
        return GeminiError("מפתח ה-API של Gemini לא תקין. בדוק את הערך ב-.env.".into());
    }
    if code == 403 {
        return GeminiError(format!("אין הרשאה לשימוש ב-Gemini עם המפתח הזה: {}", detail));
    }
    if code == 429 {
        let retry = retry_hint(&detail);
        let again = if retry.is_empty() { " נסה שוב מאוחר יותר.".to_string() } else { format!(" אפשר לנסות שוב בעוד {}.", retry) };
        let limit = regex::Regex::new(r"limit: (\d+)").unwrap().captures(&detail).map(|c| c[1].to_string());
        if detail.contains("free_tier") {
            let quota = limit.map(|l| format!(" ({} בקשות ביום למודל)", l)).unwrap_or_default();
            return GeminiError(format!("חרגת ממכסת הבקשות של החשבון החינמי ב-Google AI Studio{}.{}", quota, again));
        }
        return GeminiError(format!("חריגה ממכסת השימוש של Gemini.{}", again));
    }
    if matches!(code, 500 | 502 | 503 | 504) {
        return GeminiError("השירות של Gemini עמוס כרגע. נסה שוב בעוד רגע.".into());
    }
    GeminiError(format!("שגיאה מ-Gemini ({}): {}", code, detail))
}

/// Network failures (no HTTP answer) reported the same way.
fn network_error(e: reqwest::Error, key: &str) -> GeminiError {
    GeminiError(format!("לא ניתן להתחבר ל-Gemini: {}", e.to_string().replace(key, "***")))
}

// ---------------------------------------------------------------- mime types (Python's mimetypes.guess_type)

fn splitext(name: &str) -> (&str, &str) {
    let base_start = name.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let base = &name[base_start..];
    let leading_dots = base.len() - base.trim_start_matches('.').len();
    match base[leading_dots..].rfind('.') {
        Some(dot) => {
            let at = base_start + leading_dots + dot;
            (&name[..at], &name[at..])
        }
        None => (name, ""),
    }
}

/// Mime type the SDK would have sent for this file name (None: the SDK refused to upload it).
pub fn guess_mime_type(file_name: &str) -> Option<&'static str> {
    let mut path = file_name.to_string();
    let (mut base, mut ext) = {
        let (b, e) = splitext(&path);
        (b.to_string(), e.to_string())
    };
    while let Some((_, to)) = MIME_SUFFIX_MAP.iter().find(|(from, _)| *from == ext.to_lowercase()) {
        path = format!("{}{}", base, to);
        let (b, e) = splitext(&path);
        base = b.to_string();
        ext = e.to_string();
    }
    if MIME_ENCODINGS_MAP.contains(&ext.as_str()) {
        let (b, e) = splitext(&base);
        ext = e.to_string();
        let _ = b;
    }
    let ext = ext.to_lowercase();
    MIME_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t)
}

// ---------------------------------------------------------------- client

pub struct UploadedFile {
    pub name: String,
    pub uri: String,
    pub mime_type: String,
    pub state: Option<String>,
}

pub struct Client {
    http: reqwest::Client,
    key: String,
    base: String,
}

fn user_agent() -> String {
    format!("sing-along-studio/{}", env!("CARGO_PKG_VERSION"))
}

impl Client {
    pub fn new(key: String) -> Client {
        // GOOGLE_GEMINI_BASE_URL is honored as the SDK did (used to test against a stand-in server)
        let base = std::env::var("GOOGLE_GEMINI_BASE_URL").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| "https://generativelanguage.googleapis.com".into());
        let http = reqwest::Client::builder().user_agent(user_agent()).build().expect("HTTP client");
        Client { http, key, base: base.trim_end_matches('/').to_string() }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    async fn api_error(response: reqwest::Response) -> ApiError {
        let code = response.status().as_u16();
        let status_text = response.status().canonical_reason().unwrap_or("").to_string();
        let body = response.text().await.unwrap_or_default();
        let parsed: Option<Value> = serde_json::from_str(&body).ok();
        let err = parsed.as_ref().and_then(|v| v.get("error")).cloned().or(parsed.clone());
        let message = err.as_ref().and_then(|e| e.get("message")).and_then(Value::as_str).map(String::from);
        let status = err.as_ref().and_then(|e| e.get("status")).and_then(Value::as_str).unwrap_or(&status_text).to_string();
        let message = message.filter(|m| !m.is_empty()).unwrap_or_else(|| format!("{} {}. {}", code, status, body));
        ApiError { code, message }
    }

    async fn send_json(&self, method: reqwest::Method, url: &str, body: Option<&Value>) -> Result<Result<Value, ApiError>, reqwest::Error> {
        let mut req = self.http.request(method, url).header("x-goog-api-key", &self.key).header("Content-Type", "application/json");
        if let Some(b) = body {
            req = req.body(py::json_dumps(b));
        }
        let response = req.send().await?;
        if !response.status().is_success() {
            return Ok(Err(Self::api_error(response).await));
        }
        let text = response.text().await?;
        Ok(Ok(serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    /// generate_content, retrying a few times when Google reports it is busy (HTTP 5xx).
    pub async fn generate_content(&self, body: &Value) -> Result<Value, GeminiError> {
        let url = format!("{}/v1beta/models/{}:generateContent", self.base, GEMINI_MODEL);
        let mut attempt = 0;
        loop {
            match self.send_json(reqwest::Method::POST, &url, Some(body)).await {
                Err(e) => return Err(network_error(e, &self.key)),
                Ok(Ok(v)) => return Ok(v),
                Ok(Err(api)) if api.code >= 500 && attempt < BUSY_RETRIES => {
                    tokio::time::sleep(Duration::from_secs(BUSY_RETRY_DELAY_SECONDS * (attempt as u64 + 1))).await;
                    attempt += 1;
                }
                Ok(Err(api)) => return Err(friendly_api_error(&api, &self.key)),
            }
        }
    }

    fn file_info(v: &Value) -> UploadedFile {
        let f = v.get("file").unwrap_or(v);
        let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        UploadedFile { name: s("name"), uri: s("uri"), mime_type: s("mimeType"), state: f.get("state").and_then(Value::as_str).map(String::from) }
    }

    /// Upload a file with the resumable protocol, in 8 MB chunks (as files.upload did).
    pub async fn upload(&self, path: &Path, mime_type: &str) -> Result<UploadedFile, GeminiError> {
        let data = tokio::fs::read(path).await.map_err(|e| GeminiError(e.to_string()))?;
        let size = data.len();
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let headers = |req: reqwest::RequestBuilder| {
            req.header("x-goog-api-key", &self.key)
                .header("Content-Type", "application/json")
                .header("X-Goog-Upload-Protocol", "resumable")
                .header("X-Goog-Upload-Header-Content-Length", size.to_string())
                .header("X-Goog-Upload-Header-Content-Type", mime_type)
                .header("X-Goog-Upload-File-Name", file_name.clone())
        };
        let start_body = json!({"file": {"mime_type": mime_type, "size_bytes": size}});
        let response = headers(self.http.post(format!("{}/upload/v1beta/files", self.base)))
            .header("X-Goog-Upload-Command", "start")
            .body(py::json_dumps(&start_body))
            .send()
            .await
            .map_err(|e| network_error(e, &self.key))?;
        if !response.status().is_success() {
            return Err(friendly_api_error(&Self::api_error(response).await, &self.key));
        }
        let upload_url = response
            .headers()
            .get("x-goog-upload-url")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| GeminiError("Failed to create file. Upload URL did not returned from the create file request.".into()))?;

        let mut offset = 0usize;
        let mut last: reqwest::Response;
        loop {
            let chunk = &data[offset..(offset + UPLOAD_CHUNK).min(size)];
            let command = if offset + chunk.len() >= size { "upload, finalize" } else { "upload" };
            let mut tries = 0;
            let response = loop {
                let r = headers(self.http.post(&upload_url))
                    .header("X-Goog-Upload-Command", command)
                    .header("X-Goog-Upload-Offset", offset.to_string())
                    .body(chunk.to_vec())
                    .send()
                    .await
                    .map_err(|e| network_error(e, &self.key))?;
                if r.headers().contains_key("x-goog-upload-status") || tries >= 3 {
                    break r;
                }
                tries += 1;
                tokio::time::sleep(Duration::from_secs(1 << tries)).await;
            };
            offset += chunk.len();
            let status = response.headers().get("x-goog-upload-status").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            last = response;
            if status != "active" || offset >= size {
                break;
            }
        }
        let response = last;
        if !response.status().is_success() {
            return Err(friendly_api_error(&Self::api_error(response).await, &self.key));
        }
        let status = response.headers().get("x-goog-upload-status").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if status != "final" {
            return Err(GeminiError("Failed to upload file: Upload status is not finalized.".into()));
        }
        let text = response.text().await.map_err(|e| network_error(e, &self.key))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Self::file_info(&v))
    }

    pub async fn get_file(&self, name: &str) -> Result<UploadedFile, GeminiError> {
        match self.send_json(reqwest::Method::GET, &format!("{}/v1beta/{}", self.base, name), None).await {
            Err(e) => Err(network_error(e, &self.key)),
            Ok(Err(api)) => Err(friendly_api_error(&api, &self.key)),
            Ok(Ok(v)) => Ok(Self::file_info(&v)),
        }
    }

    pub async fn delete_file(&self, name: &str) {
        let _ = self.send_json(reqwest::Method::DELETE, &format!("{}/v1beta/{}", self.base, name), None).await;
    }

    /// Google processes an uploaded file before it can be used; poll until it is ready.
    pub async fn wait_until_active(&self, mut file: UploadedFile) -> Result<UploadedFile, GeminiError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(FILE_PROCESSING_TIMEOUT_SECONDS);
        while file.state.as_deref() == Some("PROCESSING") {
            if std::time::Instant::now() > deadline {
                return Err(GeminiError("Google לא סיים לעבד את הקובץ בזמן. נסה שוב.".into()));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            file = self.get_file(&file.name).await?;
        }
        if file.state.as_deref() == Some("FAILED") {
            return Err(GeminiError("Google לא הצליח לעבד את הקובץ.".into()));
        }
        Ok(file)
    }
}

/// The answer's text, as the SDK's response.text: all text parts of the first candidate except thoughts; None if none.
pub fn response_text(response: &Value) -> Option<String> {
    let parts = response.get("candidates")?.get(0)?.get("content")?.get("parts")?.as_array()?;
    let mut text = String::new();
    let mut any = false;
    for p in parts {
        if let Some(t) = p.get("text").and_then(Value::as_str) {
            if p.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            any = true;
            text.push_str(t);
        }
    }
    any.then_some(text)
}

/// A request's contents as the SDK built them from a list of strings and uploaded files: one user turn.
pub enum Part<'a> {
    Text(String),
    File(&'a UploadedFile),
}

pub fn user_contents(parts: &[Part]) -> Value {
    let parts: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => json!({"text": t}),
            Part::File(f) => json!({"fileData": {"file_uri": f.uri, "mime_type": f.mime_type}}),
        })
        .collect();
    json!([{"parts": parts, "role": "user"}])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_match_python() {
        assert_eq!(guess_mime_type("video.mp4"), Some("video/mp4"));
        assert_eq!(guess_mime_type("karaoke.m4a"), Some("audio/mp4"));
        assert_eq!(guess_mime_type("singer.mp3"), Some("audio/mpeg"));
        assert_eq!(guess_mime_type("karaoke.MP3"), Some("audio/mpeg"));
        assert_eq!(guess_mime_type("karaoke.flac"), Some("audio/x-flac"));
        assert_eq!(guess_mime_type("karaoke.mpga"), None);
        assert_eq!(guess_mime_type(".mp3"), None);
    }

    #[test]
    fn retry_hint_matches_python() {
        assert_eq!(retry_hint("Please retry in 12h7m28.7s."), "12 שעות ו-7 דקות");
        assert_eq!(retry_hint("Please retry in 28.7s."), "");
    }
}
