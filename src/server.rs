//! The HTTP API and the web UI: the same routes, bodies and status codes as the Python app's FastAPI main.py.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path as UrlPath, Query, Request};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::config::paths;
use crate::py::{self, Dict};
use crate::{gemini_lyrics, gemini_timing, pptx, sample, storage, video, youtube};

#[derive(RustEmbed)]
#[folder = "app/static"]
struct Assets;

/// A FastAPI-style error: {"detail": message}.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"detail": self.1}))).into_response()
    }
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}
fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, msg.into())
}
fn internal(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, msg.into())
}
fn unprocessable(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg.into())
}

type ApiResult<T> = Result<T, ApiError>;

fn dict_response(d: Dict) -> Json<Value> {
    Json(Value::Object(d))
}

// ---------------------------------------------------------------- request bodies

/// A JSON object body (FastAPI answered 422 for anything else).
fn parse_body(bytes: &[u8]) -> ApiResult<Dict> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(unprocessable("Input should be a valid dictionary")),
        Err(e) => Err(unprocessable(format!("JSON decode error: {}", e))),
    }
}

fn required_str(body: &Dict, key: &str) -> ApiResult<String> {
    match body.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(unprocessable(format!("Field required: {}", key))),
    }
}

fn optional_str(body: &Dict, key: &str) -> String {
    body.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A text field of a multipart form, and the first file named `file_field`.
struct Form {
    fields: Map<String, Value>,
    file: Option<(String, Vec<u8>)>,
}

async fn read_form(mut multipart: Multipart, file_field: &str) -> ApiResult<Form> {
    let mut form = Form { fields: Map::new(), file: None };
    while let Some(field) = multipart.next_field().await.map_err(|e| bad_request(e.to_string()))? {
        let name = field.name().unwrap_or("").to_string();
        if name == file_field {
            let filename = field.file_name().unwrap_or("").to_string();
            let data = field.bytes().await.map_err(|e| bad_request(e.to_string()))?;
            if !filename.is_empty() && form.file.is_none() {
                form.file = Some((filename, data.to_vec()));
            }
        } else {
            let text = field.text().await.map_err(|e| bad_request(e.to_string()))?;
            form.fields.insert(name, Value::String(text));
        }
    }
    Ok(form)
}

/// Python's Path(name).suffix
fn suffix(name: &str) -> String {
    Path::new(name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default()
}

// ---------------------------------------------------------------- songs

#[derive(Deserialize)]
struct ListQuery {
    search: Option<String>,
    language: Option<String>,
}

async fn list_songs(Query(q): Query<ListQuery>) -> Json<Value> {
    let songs = storage::list_songs(q.search.as_deref(), q.language.as_deref());
    Json(Value::Array(songs.into_iter().map(Value::Object).collect()))
}

async fn get_song(UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    storage::get_song(&id).map(dict_response).ok_or_else(|| not_found("Song not found"))
}

async fn save_song(multipart: Multipart) -> ApiResult<Json<Value>> {
    let form = read_form(multipart, "audio_file").await?;
    let Some(Value::String(song_json)) = form.fields.get("song_json") else {
        return Err(unprocessable("Field required: song_json"));
    };
    let data = match serde_json::from_str::<Value>(song_json) {
        Ok(Value::Object(m)) => m,
        Ok(_) => return Err(internal("Internal Server Error")),
        Err(e) => return Err(bad_request(format!("Invalid JSON data: {}", e))),
    };
    let audio = form.file.map(|(name, bytes)| {
        let ext = suffix(&name);
        (bytes, if ext.is_empty() { ".mp3".to_string() } else { ext })
    });
    let saved = tokio::task::spawn_blocking(move || storage::save_song(data, audio.as_ref().map(|(b, e)| (b.as_slice(), e.as_str()))))
        .await
        .map_err(|e| internal(e.to_string()))?
        .map_err(|e| internal(e.to_string()))?;
    Ok(dict_response(saved))
}

async fn delete_song(UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    if storage::delete_song(&id) {
        Ok(Json(json!({"status": "deleted", "id": id})))
    } else {
        Err(not_found("Song not found"))
    }
}

/// A file response with Range support (the audio player seeks), as Starlette's FileResponse.
async fn file_response(path: &Path, media_type: &str, filename: Option<&str>, request_headers: &HeaderMap) -> ApiResult<Response> {
    let data = tokio::fs::read(path).await.map_err(|_| not_found("File not found"))?;
    let total = data.len();
    let mut builder = Response::builder().header(header::CONTENT_TYPE, media_type).header(header::ACCEPT_RANGES, "bytes");
    if let Some(name) = filename {
        let disposition = if name.is_ascii() {
            format!("attachment; filename=\"{}\"", name.replace('"', "\\\""))
        } else {
            format!("attachment; filename*=utf-8''{}", py::quote(name))
        };
        builder = builder.header(header::CONTENT_DISPOSITION, disposition);
    }
    let range = request_headers.get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("bytes=")).and_then(|v| {
        let (a, b) = v.split_once('-')?;
        let start: usize = if a.is_empty() { total.saturating_sub(b.parse::<usize>().ok()?) } else { a.parse().ok()? };
        let end: usize = if a.is_empty() || b.is_empty() { total.saturating_sub(1) } else { b.parse::<usize>().ok()?.min(total.saturating_sub(1)) };
        (start <= end && start < total).then_some((start, end))
    });
    let response = match range {
        Some((start, end)) => builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total))
            .header(header::CONTENT_LENGTH, end - start + 1)
            .body(Body::from(data[start..=end].to_vec())),
        None => builder.header(header::CONTENT_LENGTH, total).body(Body::from(data)),
    };
    response.map_err(|e| internal(e.to_string()))
}

async fn song_audio(UrlPath(id): UrlPath<String>, headers: HeaderMap) -> ApiResult<Response> {
    let song = storage::get_song(&id);
    let Some(audio_path) = song.as_ref().and_then(|s| s.get("audio_path")).and_then(Value::as_str).filter(|p| !p.is_empty()) else {
        return Err(not_found("Audio file not found"));
    };
    let path = PathBuf::from(audio_path);
    if !path.exists() {
        return Err(not_found("Audio file missing on disk"));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    file_response(&path, "audio/mpeg", name.as_deref(), &headers).await
}

async fn song_lrc(UrlPath(id): UrlPath<String>, headers: HeaderMap) -> ApiResult<Response> {
    let path = storage::lrc_path(&id);
    if !path.exists() {
        return Err(not_found("LRC file not found"));
    }
    file_response(&path, "text/plain; charset=utf-8", Some(&format!("{}.lrc", id)), &headers).await
}

// ---------------------------------------------------------------- audio from a link, and AI

async fn audio_from_url(body: axum::body::Bytes) -> ApiResult<Response> {
    let body = parse_body(&body)?;
    let url = required_str(&body, "url")?;
    let (mp3, info) = youtube::download_audio_mp3(&url).await.map_err(|e| bad_request(e.0))?;
    // Titles may be Hebrew, so send metadata percent-encoded in headers
    Response::builder()
        .header(header::CONTENT_TYPE, "audio/mpeg")
        .header("X-Audio-Title", py::quote(&info.title))
        .header("X-Audio-Artist", py::quote(&info.artist))
        .body(Body::from(mp3))
        .map_err(|e| internal(e.to_string()))
}

fn string_list(v: Option<&Value>) -> Option<Vec<String>> {
    match v {
        Some(Value::Array(items)) => items.iter().map(|i| i.as_str().map(String::from)).collect(),
        _ => None,
    }
}

async fn ai_time_verses(body: axum::body::Bytes) -> ApiResult<Json<Value>> {
    let body = parse_body(&body)?;
    let url = required_str(&body, "youtube_url")?;
    let verses = string_list(body.get("verses")).ok_or_else(|| unprocessable("Input should be a valid list of strings: verses"))?;
    let result = gemini_timing::time_verses_from_youtube(&url, &verses, &optional_str(&body, "title"), &optional_str(&body, "artist"))
        .await
        .map_err(|e| bad_request(e.0))?;
    Ok(dict_response(result))
}

async fn ai_find_lyrics(body: axum::body::Bytes) -> ApiResult<Json<Value>> {
    let body = parse_body(&body)?;
    let (title, artist) = (required_str(&body, "title")?, required_str(&body, "artist")?);
    gemini_lyrics::find_lyrics(&title, &artist).await.map(dict_response).map_err(|e| bad_request(e.0))
}

async fn ai_time_verses_singer(multipart: Multipart) -> ApiResult<Json<Value>> {
    let form = read_form(multipart, "karaoke_audio").await?;
    let field = |k: &str| form.fields.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let singer_url = form.fields.get("singer_url").and_then(Value::as_str).ok_or_else(|| unprocessable("Field required: singer_url"))?.to_string();
    let verses_raw = form.fields.get("verses").and_then(Value::as_str).ok_or_else(|| unprocessable("Field required: verses"))?;
    let verses = serde_json::from_str::<Value>(verses_raw).ok().and_then(|v| string_list(Some(&v))).ok_or_else(|| bad_request("רשימת הבתים אינה תקינה"))?;

    let work = tempfile::tempdir().map_err(|e| internal(e.to_string()))?;
    let karaoke_path = match &form.file {
        Some((name, bytes)) => {
            let ext = suffix(name);
            let path = work.path().join(format!("karaoke{}", if ext.is_empty() { ".mp3" } else { &ext }));
            std::fs::write(&path, bytes).map_err(|e| internal(e.to_string()))?;
            path
        }
        None => {
            let song = form.fields.get("song_id").and_then(Value::as_str).filter(|s| !s.is_empty()).and_then(storage::get_song);
            match song.as_ref().and_then(|s| s.get("audio_path")).and_then(Value::as_str).filter(|p| !p.is_empty()) {
                Some(p) => PathBuf::from(p),
                None => return Err(bad_request("לשיר אין קובץ שמע. הוסף קובץ שמע או שמור את השיר קודם.")),
            }
        }
    };
    gemini_timing::time_verses_from_singer_version(&singer_url, &karaoke_path, &verses, &field("title"), &field("artist"))
        .await
        .map(dict_response)
        .map_err(|e| bad_request(e.0))
}

// ---------------------------------------------------------------- performances

async fn list_performances() -> Json<Value> {
    Json(Value::Array(storage::list_performances().into_iter().map(Value::Object).collect()))
}

async fn get_performance(UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    storage::get_performance(&id).map(dict_response).ok_or_else(|| not_found("Performance not found"))
}

async fn save_performance(body: axum::body::Bytes) -> ApiResult<Json<Value>> {
    let perf = parse_body(&body)?;
    let saved = tokio::task::spawn_blocking(move || storage::save_performance(perf))
        .await
        .map_err(|e| internal(e.to_string()))?
        .map_err(|e| internal(e.to_string()))?;
    Ok(dict_response(saved))
}

async fn delete_performance(UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    if storage::delete_performance(&id) {
        Ok(Json(json!({"status": "deleted", "id": id})))
    } else {
        Err(not_found("Performance not found"))
    }
}

// ---------------------------------------------------------------- exports

/// A boolean as pydantic read it: true/false, 0/1, or strings such as "yes", "off", "true".
fn py_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.as_f64()? {
            x if x == 1.0 => Some(true),
            x if x == 0.0 => Some(false),
            _ => None,
        },
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "1" | "t" | "true" | "y" | "yes" | "on" => Some(true),
            "0" | "f" | "false" | "n" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn opt_id(body: &Dict, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

async fn export_pptx(body: axum::body::Bytes) -> ApiResult<Json<Value>> {
    let body = parse_body(&body)?;
    // Each song with audio waits for a click on its title slide (on unless turned off)
    let wait_for_click = match body.get("wait_for_click") {
        None => true,
        Some(v) => py_bool(v).ok_or_else(|| unprocessable("Input should be a valid boolean: wait_for_click"))?,
    };
    let (songs, title, prefix, theme): (Vec<Dict>, String, String, Option<Dict>);
    if let Some(pid) = opt_id(&body, "performance_id") {
        let perf = storage::get_performance(&pid).ok_or_else(|| not_found("Performance not found"))?;
        songs = storage::performance_songs(&perf);
        title = py::get_str(&perf, "title", "שירה בציבור").to_string();
        prefix = py::get_str(&perf, "id", "performance").to_string();
        theme = Some(perf);
    } else if let Some(sid) = opt_id(&body, "song_id") {
        let song = storage::get_song(&sid).ok_or_else(|| not_found("Song not found"))?;
        title = py::get_str(&song, "title", "").to_string();
        prefix = py::get_str(&song, "id", "song").to_string();
        songs = vec![song];
        theme = None;
    } else {
        return Err(bad_request("Provide either performance_id or song_id"));
    }
    if songs.is_empty() {
        return Err(bad_request("No songs to export"));
    }
    let count = songs.len();
    let path = tokio::task::spawn_blocking(move || pptx::generate_presentation(&songs, &title, &prefix, theme.as_ref(), wait_for_click))
        .await
        .map_err(|e| internal(e.to_string()))?
        .map_err(internal)?;
    let filename = file_name_of(&path);
    Ok(Json(json!({
        "success": true,
        "filename": filename,
        "download_url": format!("/api/downloads/presentations/{}", filename),
        "song_count": count,
    })))
}

async fn export_video(body: axum::body::Bytes) -> ApiResult<Json<Value>> {
    let body = parse_body(&body)?;
    // A number = constant frame rate in frames per second. An explicit null = variable frame rate. Left out: the default.
    let requested = match body.get("frame_rate") {
        None => Some(video::DEFAULT_FRAME_RATE),
        Some(Value::Null) => None,
        Some(v) => Some(py::float(v).ok_or_else(|| unprocessable("Input should be a valid number: frame_rate"))?),
    };
    let frame_rate = video::check_frame_rate(requested).map_err(bad_request)?;

    if let Some(pid) = opt_id(&body, "performance_id") {
        let perf = storage::get_performance(&pid).ok_or_else(|| not_found("Performance not found"))?;
        let songs = storage::performance_songs(&perf);
        if songs.is_empty() {
            return Err(bad_request("Performance contains no songs"));
        }
        let (prefix, perf_title, count) = (py::get_str(&perf, "id", "performance").to_string(), perf.get("title").cloned().unwrap_or(Value::Null), songs.len());
        let path = tokio::task::spawn_blocking(move || video::generate_performance_video(&songs, &prefix, Some(&perf), frame_rate))
            .await
            .map_err(|e| internal(e.to_string()))?
            .map_err(internal)?;
        let filename = file_name_of(&path);
        Ok(Json(json!({
            "success": true,
            "filename": filename,
            "download_url": format!("/api/downloads/videos/{}", filename),
            "title": perf_title,
            "song_count": count,
        })))
    } else if let Some(sid) = opt_id(&body, "song_id") {
        let song = storage::get_song(&sid).ok_or_else(|| not_found("Song not found"))?;
        let song_title = song.get("title").cloned().unwrap_or(Value::Null);
        let prefix = py::get_str(&song, "id", "karaoke").to_string();
        let path = tokio::task::spawn_blocking(move || video::generate_karaoke_video(&song, &prefix, None, frame_rate))
            .await
            .map_err(|e| internal(e.to_string()))?
            .map_err(internal)?;
        let filename = file_name_of(&path);
        Ok(Json(json!({
            "success": true,
            "filename": filename,
            "download_url": format!("/api/downloads/videos/{}", filename),
            "song_title": song_title,
        })))
    } else {
        Err(bad_request("Provide either song_id or performance_id"))
    }
}

async fn download_file(UrlPath((folder, filename)): UrlPath<(String, String)>, headers: HeaderMap) -> ApiResult<Response> {
    let (target, media_type) = match folder.as_str() {
        "presentations" => (paths().presentations.join(&filename), "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "videos" => (paths().videos.join(&filename), "video/mp4"),
        _ => return Err(bad_request("Invalid folder")),
    };
    if !target.exists() {
        return Err(not_found("File not found"));
    }
    file_response(&target, media_type, Some(&filename), &headers).await
}

// ---------------------------------------------------------------- static UI

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn no_cache(mut response: Response) -> Response {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

async fn static_file(UrlPath(path): UrlPath<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => no_cache(([(header::CONTENT_TYPE, mime_for(&path))], file.data.into_owned()).into_response()),
        None => no_cache((StatusCode::NOT_FOUND, Json(json!({"detail": "Not Found"}))).into_response()),
    }
}

async fn index() -> Response {
    let Some(file) = Assets::get("index.html") else { return StatusCode::NOT_FOUND.into_response() };
    let mut html = String::from_utf8_lossy(&file.data).into_owned();
    // Put each asset's last-modified time in its address, so a changed file is always fetched fresh
    for asset in ["css/style.css", "js/app.js"] {
        let version = Assets::get(asset).and_then(|f| f.metadata.last_modified()).unwrap_or(0);
        html = html.replace(&format!("/static/{}", asset), &format!("/static/{}?v={}", asset, version));
    }
    no_cache(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response())
}

/// CORS as the Python app configured it: everything allowed.
async fn cors(request: Request, next: Next) -> Response {
    let preflight = request.method() == axum::http::Method::OPTIONS;
    let origin = request.headers().get(header::ORIGIN).cloned();
    let mut response = if preflight { StatusCode::OK.into_response() } else { next.run(request).await };
    let h = response.headers_mut();
    if let Some(origin) = origin {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        h.insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HeaderValue::from_static("true"));
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    if preflight {
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("*"));
    }
    response
}

/// Everything the server offers; `extra` lets the desktop app add its own routes.
pub fn router(extra: Option<Router>) -> Router {
    let mut app = Router::new()
        .route("/api/songs", get(list_songs).post(save_song))
        .route("/api/songs/{id}", get(get_song).delete(delete_song))
        .route("/api/songs/{id}/audio", get(song_audio))
        .route("/api/songs/{id}/lrc", get(song_lrc))
        .route("/api/audio/from-url", post(audio_from_url))
        .route("/api/ai/time-verses", post(ai_time_verses))
        .route("/api/ai/find-lyrics", post(ai_find_lyrics))
        .route("/api/ai/time-verses-singer", post(ai_time_verses_singer))
        .route("/api/performances", get(list_performances).post(save_performance))
        .route("/api/performances/{id}", get(get_performance).delete(delete_performance))
        .route("/api/export/pptx", post(export_pptx))
        .route("/api/export/video", post(export_video))
        .route("/api/downloads/{folder}/{filename}", get(download_file))
        .route("/static/{*path}", get(static_file))
        .route("/", get(index));
    if let Some(extra) = extra {
        app = app.merge(extra);
    }
    app.layer(DefaultBodyLimit::disable()).layer(middleware::from_fn(cors))
}

/// Create the folders and the sample songs (the Python app's startup event).
pub fn startup() {
    crate::config::ensure_dirs();
    sample::seed_sample_data_if_empty();
}

/// Serve on an address until the process ends.
pub async fn serve(listener: tokio::net::TcpListener, extra: Option<Router>) -> std::io::Result<()> {
    axum::serve(listener, router(extra)).await
}
