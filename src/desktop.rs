//! The desktop app: the server runs on a free local port and the page is shown in a native window (Edge WebView2).
//!
//! WebView2 does not download files, so exports go through a native Save As dialog instead. The page calls
//! window.pywebview.api.save_export(folder, filename) when it exists (kept from the Python app so the page needs no
//! changes); here it is a small script that asks a desktop-only server route, which shows the dialog and copies the file.

use std::path::Path;

use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder};

use crate::config::{self, paths, APP_NAME};
use crate::{log, server};

const SHIM: &str = r#"
window.pywebview = { api: {
  save_export: async (folder, filename) => {
    const res = await fetch("/__desktop/save_export", {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ folder, filename })
    });
    const data = await res.json();
    if (!res.ok) throw new Error(data.detail || "save failed");
    return data.path;
  }
} };
"#;

/// Copy a finished export to a place the user picks. Returns the saved path, or None if cancelled.
fn save_export(folder: &str, filename: &str) -> Result<Option<String>, String> {
    let dir = match folder {
        "presentations" => &paths().presentations,
        "videos" => &paths().videos,
        _ => return Err("Invalid export".into()),
    };
    if Path::new(filename).file_name().map(|n| n.to_string_lossy() != filename).unwrap_or(true) {
        return Err("Invalid export".into());
    }
    let source = dir.join(filename);
    if !source.is_file() {
        return Err(format!("{}: not found", filename));
    }
    let mut dialog = rfd::FileDialog::new().set_file_name(filename);
    if let Some(ext) = Path::new(filename).extension() {
        dialog = dialog.add_filter(ext.to_string_lossy().to_uppercase(), &[ext.to_string_lossy().to_string()]);
    }
    let Some(target) = dialog.save_file() else { return Ok(None) };
    std::fs::copy(&source, &target).map_err(|e| e.to_string())?;
    Ok(Some(target.to_string_lossy().into_owned()))
}

async fn save_export_route(Json(body): Json<Value>) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    let folder = body.get("folder").and_then(Value::as_str).unwrap_or("").to_string();
    let filename = body.get("filename").and_then(Value::as_str).unwrap_or("").to_string();
    // The dialog blocks, so keep it off the async threads
    let result = tokio::task::spawn_blocking(move || save_export(&folder, &filename)).await.map_err(|e| e.to_string()).and_then(|r| r);
    match result {
        Ok(path) => Ok(Json(json!({"path": path}))),
        Err(e) => Err((axum::http::StatusCode::BAD_REQUEST, Json(json!({"detail": e})))),
    }
}

/// The window and taskbar icon, decoded from the embedded PNG.
fn window_icon() -> Option<tao::window::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/icon.png")));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    tao::window::Icon::from_rgba(buf, info.width, info.height).ok()
}

/// First run: give the user a .env to edit (Gemini key, data/exports folders).
fn prepare_home(home: &Path) {
    let _ = std::fs::create_dir_all(home);
    let env_file = home.join(".env");
    if !env_file.exists() {
        let _ = std::fs::write(&env_file, include_str!("../.env.example"));
    }
    // A windowed exe has no console: log to a file
    log::to_file(&home.join("logs").join("app.log"));
}

pub fn run() {
    let home = config::default_home(true);
    prepare_home(&home);
    config::init(home.clone());

    // Serve on a free local port, in a thread of its own
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free local port");
    let port = listener.local_addr().expect("local address").port();
    listener.set_nonblocking(true).expect("non-blocking listener");
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("async runtime");
        rt.block_on(async move {
            server::startup();
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            let extra = Router::new().route("/__desktop/save_export", post(save_export_route));
            let _ = server::serve(listener, Some(extra)).await;
        });
    });
    // Wait until it answers
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let event_loop = EventLoopBuilder::new().build();
    let window = WindowBuilder::new()
        .with_title(format!("{} | אולפן שירה בציבור וקריוקי", APP_NAME))
        .with_window_icon(window_icon())
        .with_inner_size(LogicalSize::new(1400.0, 900.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .build(&event_loop)
        .expect("window");
    // WebView2's own files stay with the app's data, so nothing is left elsewhere
    let mut context = WebContext::new(Some(home.join("webview")));
    let _webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(format!("http://127.0.0.1:{}/", port))
        .with_initialization_script(SHIM)
        .build(&window)
        .expect("WebView2 (the Edge WebView2 Runtime is needed; it ships with Windows 11)");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}
