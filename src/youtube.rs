//! Audio and video from YouTube links, through yt-dlp.
//!
//! yt-dlp and a small JavaScript runtime it needs for YouTube (QuickJS-ng) are downloaded the first time a link is
//! used, into <home>/bin, and verified against known checksums. yt-dlp keeps itself current (`yt-dlp -U`): once a
//! week, and right away when YouTube refuses a download. There is no FFmpeg: yt-dlp downloads the streams as they
//! are and this app converts the audio to MP3 and joins a video's separate video and audio streams itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::config::paths;
use crate::log;

/// Songs are never this long; the cap also stops accidental livestream/hour-long mix downloads
pub const MAX_DURATION_SECONDS: u64 = 20 * 60;

/// YouTube serves audio through several "clients", and yt-dlp picks them per request. Some of them hand out stream
/// URLs that are refused with HTTP 403, so a failed download is retried through these alternatives (None = yt-dlp's
/// own choice).
const PLAYER_CLIENT_ATTEMPTS: [Option<&str>; 3] = [None, Some("default,-android_vr"), Some("mweb")];

/// Errors worth retrying with another client; anything else (private/removed video...) fails right away
const RETRYABLE: [&str; 4] = ["403", "forbidden", "requested format is not available", "page needs to be reloaded"];

/// Small video with its audio track: Gemini counts video tokens per second regardless of resolution,
/// so a low resolution only saves download/upload time. Prefer MP4 (H.264 + AAC), which Gemini accepts.
const VIDEO_FORMAT: &str = "bv*[height<=360][vcodec^=avc1]+ba[ext=m4a]/bv*[height<=360][ext=mp4]+ba[ext=m4a]/b[height<=360][ext=mp4]/bv*[height<=360]+ba/b[height<=360]/w";

/// Audio: YouTube's AAC stream when it has one (converted to MP3 here; Opus would need a decoder this app lacks)
const AUDIO_FORMAT: &str = "bestaudio[ext=m4a]/bestaudio[ext=mp4]/bestaudio/best";

/// yt-dlp updates itself at most this often (plus whenever YouTube refuses a download)
const UPDATE_INTERVAL: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug)]
pub struct AudioDownloadError(pub String);

pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub duration: f64,
}

// ---------------------------------------------------------------- the helper programs

struct Tool {
    file: &'static str,
    url: &'static str,
    /// Pinned SHA-256, or None to check against yt-dlp's published SHA2-256SUMS
    sha256: Option<&'static str>,
}

#[cfg(windows)]
const TOOLS: [Tool; 2] = [
    Tool { file: "yt-dlp.exe", url: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe", sha256: None },
    Tool { file: "qjs.exe", url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-windows-x86_64.exe", sha256: Some("2aeabf0092c3262d6b2609824418f7dd7ed1f1df939f73b2b15645230cac0d77") },
];
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TOOLS: [Tool; 2] = [
    Tool { file: "yt-dlp", url: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux", sha256: None },
    Tool { file: "qjs", url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-linux-x86_64", sha256: Some("0bfc02511a9f549c28b53880d988fc7cd5d361e90c5e8afdfcd7dc6774ceace5") },
];
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const TOOLS: [Tool; 2] = [
    Tool { file: "yt-dlp", url: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64", sha256: None },
    Tool { file: "qjs", url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-linux-aarch64", sha256: Some("3372133484edf50a69f3c67903af41206d22a061e930e3cfb63269272ef56d2e") },
];
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TOOLS: [Tool; 2] = [
    Tool { file: "yt-dlp", url: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos", sha256: None },
    Tool { file: "qjs", url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-darwin-arm64", sha256: Some("8be3ddfe3397d2e692e4e1e8972ee9d032a0a580505d2f8b4ea528cf1b651c11") },
];
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const TOOLS: [Tool; 2] = [
    Tool { file: "yt-dlp", url: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos", sha256: None },
    Tool { file: "qjs", url: "https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-darwin-x86_64", sha256: Some("9e5e101b4fd13cda3204222ca9f8be35412c41dcdef3745829633b7a67245412") },
];

const YTDLP_SUMS_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";

/// One download of the tools at a time, shared by all requests
static TOOLS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{:02x}", b)).collect()
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("sing-along-studio/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .expect("HTTP client")
}

async fn download(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let r = http.get(url).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {} for {}", r.status().as_u16(), url));
    }
    Ok(r.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

/// Download one tool into bin/, verified, written under a temporary name and then renamed into place.
async fn install(http: &reqwest::Client, tool: &Tool, bin: &Path) -> Result<(), String> {
    log::info(&format!("Downloading {} from {}", tool.file, tool.url));
    let data = download(http, tool.url).await?;
    let expected = match tool.sha256 {
        Some(h) => h.to_string(),
        None => {
            let sums = String::from_utf8_lossy(&download(http, YTDLP_SUMS_URL).await?).into_owned();
            let asset = tool.url.rsplit('/').next().unwrap_or(tool.file);
            sums.lines()
                .find_map(|l| {
                    let mut it = l.split_whitespace();
                    let (hash, name) = (it.next()?, it.next()?);
                    (name.trim_start_matches('*') == asset).then(|| hash.to_lowercase())
                })
                .ok_or_else(|| format!("{} is not listed in yt-dlp's checksums", asset))?
        }
    };
    if sha256_hex(&data) != expected {
        return Err(format!("{}: checksum mismatch", tool.file));
    }
    let part = bin.join(format!("{}.part", tool.file));
    std::fs::write(&part, &data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&part, bin.join(tool.file)).map_err(|e| e.to_string())?;
    Ok(())
}

struct Tools {
    ytdlp: PathBuf,
    qjs: PathBuf,
}

fn command(program: &Path) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        // No console window flashing up from the desktop app
        cmd.creation_flags(0x0800_0000);
    }
    cmd.kill_on_drop(true);
    cmd
}

/// Let yt-dlp update itself; failures are only logged (the installed copy keeps working).
async fn self_update(ytdlp: &Path) {
    let stamp = ytdlp.with_extension("updated");
    match command(ytdlp).arg("-U").arg("--no-warnings").output().await {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            log::info(&format!("yt-dlp -U: {}", text.lines().last().unwrap_or("").trim()));
        }
        Err(e) => log::warn(&format!("yt-dlp -U failed: {}", e)),
    }
    let _ = std::fs::write(stamp, b"");
}

/// The helper programs, downloading them the first time.
async fn ensure_tools() -> Result<Tools, AudioDownloadError> {
    let _guard = TOOLS_LOCK.lock().await;
    let bin = paths().bin.clone();
    std::fs::create_dir_all(&bin).map_err(|e| AudioDownloadError(e.to_string()))?;
    let http = http_client();
    for tool in &TOOLS {
        if !bin.join(tool.file).is_file() {
            install(&http, tool, &bin).await.map_err(|e| AudioDownloadError(format!("הורדת רכיב ההורדה מיוטיוב נכשלה: {}", e)))?;
            if tool.file.starts_with("yt-dlp") {
                let _ = std::fs::write(bin.join(tool.file).with_extension("updated"), b"");
            }
        }
    }
    let ytdlp = bin.join(TOOLS[0].file);
    let stale = std::fs::metadata(ytdlp.with_extension("updated"))
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default() > UPDATE_INTERVAL)
        .unwrap_or(true);
    if stale {
        self_update(&ytdlp).await;
    }
    Ok(Tools { ytdlp, qjs: bin.join(TOOLS[1].file) })
}

// ---------------------------------------------------------------- downloads

/// Return a clean http(s) link ("www.youtube.com/..." gets https://), or an error.
pub fn normalize_url(url: &str) -> Result<String, AudioDownloadError> {
    let mut url = crate::py::strip(url).to_string();
    if !url.is_empty() && !url.contains("://") {
        url = format!("https://{}", url); // allow pasting "www.youtube.com/watch?v=..."
    }
    let (scheme, rest) = url.split_once(':').unwrap_or(("", ""));
    let scheme_ok = matches!(scheme.to_lowercase().as_str(), "http" | "https");
    let netloc = rest.strip_prefix("//").map(|r| r.split(['/', '?', '#']).next().unwrap_or("")).unwrap_or("");
    if !scheme_ok || !netloc.contains('.') || url.chars().any(crate::py::is_space) {
        return Err(AudioDownloadError("הקישור אינו תקין. הדבק כתובת מלאה שמתחילה ב-https://".into()));
    }
    Ok(url)
}

/// yt-dlp colors its messages for terminals and prefixes them with "ERROR: ".
fn clean_message(stderr: &str) -> String {
    let ansi = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
    let text = ansi.replace_all(stderr, "");
    let errors: Vec<&str> = text.lines().filter(|l| l.starts_with("ERROR:")).collect();
    let chosen = if errors.is_empty() { text.trim().to_string() } else { errors.join("\n") };
    chosen.replace("ERROR: ", "").trim().to_string()
}

/// One yt-dlp attempt writing out_dir/<basename>.<ext>; returns yt-dlp's info, or None if the video was filtered out.
async fn run_ytdlp(tools: &Tools, url: &str, out_dir: &Path, basename: &str, clients: Option<&str>, format: &str) -> Result<Option<Value>, String> {
    let mut cmd = command(&tools.ytdlp);
    cmd.arg("--no-js-runtimes")
        .arg("--js-runtimes")
        .arg(format!("quickjs:{}", tools.qjs.display()))
        // Never let a system FFmpeg take part: results must not depend on what else is installed
        .arg("--ffmpeg-location")
        .arg(out_dir.join("no-ffmpeg"))
        .args(["--no-playlist", "--no-warnings", "--no-progress", "--color", "never", "--no-mtime"])
        .args(["--match-filter", &format!("duration <= {}", MAX_DURATION_SECONDS)])
        .args(["-f", format])
        .arg("-o")
        .arg(out_dir.join(format!("{}.%(ext)s", basename)))
        .args(["--no-simulate", "--print", "after_move:%()j"]);
    if let Some(c) = clients {
        cmd.arg("--extractor-args").arg(format!("youtube:player_client={}", c));
    }
    cmd.arg("--").arg(url);
    let out = cmd.output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(clean_message(&String::from_utf8_lossy(&out.stderr)));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let info = stdout.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(Value::is_object));
    Ok(info)
}

fn too_long_error() -> AudioDownloadError {
    AudioDownloadError(format!("לא ניתן להוריד את הסרטון (ייתכן שהוא ארוך מ-{} דקות)", MAX_DURATION_SECONDS / 60))
}

/// Run yt-dlp, retrying through other YouTube clients when a download is refused.
/// Each attempt gets its own subfolder of work_dir. Returns (that folder, yt-dlp's info).
async fn download_with_retries(url: &str, work_dir: &Path, basename: &str, format: &str) -> Result<(PathBuf, Value), AudioDownloadError> {
    let tools = ensure_tools().await?;
    let mut last_error = String::new();
    let mut updated = false;
    let mut attempt = 0;
    let mut clients_index = 0;
    while clients_index < PLAYER_CLIENT_ATTEMPTS.len() {
        let out_dir = work_dir.join(format!("attempt{}", attempt));
        attempt += 1;
        std::fs::create_dir_all(&out_dir).map_err(|e| AudioDownloadError(e.to_string()))?;
        match run_ytdlp(&tools, url, &out_dir, basename, PLAYER_CLIENT_ATTEMPTS[clients_index], format).await {
            Ok(Some(info)) => return Ok((out_dir, info)),
            Ok(None) => return Err(too_long_error()),
            Err(message) => {
                last_error = message;
                if !RETRYABLE.iter().any(|m| last_error.to_lowercase().contains(m)) {
                    return Err(AudioDownloadError(format!("ההורדה נכשלה: {}", last_error)));
                }
                // YouTube changes often: the first refusal is the moment to make sure yt-dlp is current
                if !updated {
                    updated = true;
                    self_update(&tools.ytdlp).await;
                    continue;
                }
                clients_index += 1;
            }
        }
    }
    Err(AudioDownloadError(format!("ההורדה נכשלה: {} (יוטיוב חסם את ההורדה. נסה שוב בעוד רגע.)", last_error)))
}

fn info_str(info: &Value, key: &str) -> String {
    info.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("").to_string()
}

fn media_info(info: &Value) -> MediaInfo {
    let track = info_str(info, "track");
    MediaInfo {
        title: if track.is_empty() { info_str(info, "title") } else { track },
        artist: info_str(info, "artist"),
        duration: info.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
    }
}

/// Downloaded files named <basename>.* in a folder (finished ones only).
fn downloaded(dir: &Path, basename: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|e| e.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        p.is_file() && name.starts_with(&format!("{}.", basename)) && !name.ends_with(".part") && !name.ends_with(".ytdl")
    });
    files.sort();
    files
}

/// Download the audio of a video link and convert it to MP3 (192 kbps). Returns (mp3 bytes, info).
pub async fn download_audio_mp3(url: &str) -> Result<(Vec<u8>, MediaInfo), AudioDownloadError> {
    let url = normalize_url(url)?;
    let work = tempfile::tempdir().map_err(|e| AudioDownloadError(e.to_string()))?;
    let (out_dir, info) = download_with_retries(&url, work.path(), "audio", AUDIO_FORMAT).await?;
    let file = downloaded(&out_dir, "audio").into_iter().next().ok_or_else(too_long_error)?;
    let mp3 = tokio::task::spawn_blocking(move || crate::audio::convert_to_mp3(&file, 192))
        .await
        .map_err(|e| AudioDownloadError(e.to_string()))?
        .map_err(|e| AudioDownloadError(format!("ההמרה ל-MP3 נכשלה: {}", e)))?;
    Ok((mp3, media_info(&info)))
}

/// Download a small version of the video (with its audio) inside work_dir. Returns (video file, info).
pub async fn download_video(url: &str, work_dir: &Path) -> Result<(PathBuf, MediaInfo), AudioDownloadError> {
    let url = normalize_url(url)?;
    let (out_dir, info) = download_with_retries(&url, work_dir, "video", VIDEO_FORMAT).await?;
    let files = downloaded(&out_dir, "video");
    let failed = || AudioDownloadError("הורדת הסרטון נכשלה: לא נוצר קובץ וידאו".into());
    let video = match files.len() {
        0 => return Err(failed()),
        1 => files[0].clone(),
        _ => {
            // Separate video and audio streams: join them into one MP4, as yt-dlp's FFmpeg merger did
            let target = out_dir.join("video.mp4");
            let (files, t) = (files.clone(), target.clone());
            tokio::task::spawn_blocking(move || crate::mp4::merge_streams(&files, &t))
                .await
                .map_err(|e| AudioDownloadError(e.to_string()))?
                .map_err(|e| AudioDownloadError(format!("הורדת הסרטון נכשלה: {}", e)))?;
            target
        }
    };
    Ok((video, media_info(&info)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(normalize_url(" www.youtube.com/watch?v=x ").unwrap(), "https://www.youtube.com/watch?v=x");
        assert!(normalize_url("ftp://x.com/a").is_err());
        assert!(normalize_url("https://localhost/a").is_err());
        assert!(normalize_url("https://x.com/a b").is_err());
    }

    #[test]
    fn messages() {
        assert_eq!(clean_message("\x1b[0;31mERROR:\x1b[0m [youtube] abc: Video unavailable\n"), "[youtube] abc: Video unavailable");
    }
}
