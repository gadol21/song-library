//! Minimal logging: lines go to stderr, or to <home>/logs/app.log when the desktop app has no console.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// Send log lines to a file instead of stderr.
pub fn to_file(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
        *FILE.lock().unwrap() = Some(f);
    }
}

fn write(level: &str, msg: &str) {
    let line = format!("{}:     {}\n", level, msg);
    let mut file = FILE.lock().unwrap();
    match file.as_mut() {
        Some(f) => {
            let _ = f.write_all(line.as_bytes());
        }
        None => {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }
}

pub fn info(msg: &str) {
    write("INFO", msg);
}

pub fn warn(msg: &str) {
    write("WARNING", msg);
}

pub fn error(msg: &str) {
    write("ERROR", msg);
}
