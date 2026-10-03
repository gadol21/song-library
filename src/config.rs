//! Where things live: the settings folder (SINGALONG_HOME), the .env in it, and the data/exports folders.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const APP_NAME: &str = "Sing-Along Studio";

pub struct Paths {
    /// Holds .env and bin/; relative data paths are relative to it
    pub home: PathBuf,
    pub songs: PathBuf,
    pub performances: PathBuf,
    pub presentations: PathBuf,
    pub videos: PathBuf,
    /// Helper programs downloaded on first use (yt-dlp and its JavaScript runtime)
    pub bin: PathBuf,
}

static PATHS: OnceLock<Paths> = OnceLock::new();

pub fn paths() -> &'static Paths {
    PATHS.get().expect("config::init must run first")
}

/// The settings folder: SINGALONG_HOME, else %APPDATA%\Sing-Along Studio for the desktop app,
/// else the current folder (the project folder when started with run.bat / run.sh).
pub fn default_home(desktop: bool) -> PathBuf {
    if let Some(home) = std::env::var_os("SINGALONG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home);
    }
    if desktop {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).or_else(home_dir).unwrap_or_else(|| PathBuf::from("."));
        return base.join(APP_NAME);
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// Like Python's Path.resolve() for a path that may not exist: absolute, with "." and ".." removed.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Folder from an environment variable; ~ is expanded and relative paths are relative to home.
fn env_dir(home: &Path, name: &str, default: &str) -> PathBuf {
    let raw = std::env::var(name).unwrap_or_default();
    let raw = raw.trim().trim_matches('"').trim_matches('\'');
    let raw = if raw.is_empty() { default } else { raw };
    let mut path = PathBuf::from(raw);
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        if let Some(h) = home_dir() {
            path = h.join(raw[1..].trim_start_matches(['/', '\\']));
        }
    }
    if path.is_absolute() {
        path
    } else {
        resolve(&home.join(path))
    }
}

/// One .env line as python-dotenv read it: `KEY=value`, optional `export `, `#` comments. An unquoted value is taken
/// as written (so Windows paths with backslashes work) up to a " #" comment; 'single quoted' is literal;
/// "double quoted" understands escapes such as \n, \t, \" and \\.
pub fn parse_env_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").map(str::trim_start).unwrap_or(line);
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let value = if let Some(body) = rest.strip_prefix('\'') {
        body.find('\'').map_or(body, |end| &body[..end]).to_string()
    } else if let Some(body) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some(o @ ('"' | '\\' | '\'')) => out.push(o),
                    Some(o) => {
                        out.push('\\');
                        out.push(o);
                    }
                    None => out.push('\\'),
                },
                c => out.push(c),
            }
        }
        out
    } else {
        let end = rest.find(" #").or_else(|| rest.find("\t#")).unwrap_or(rest.len());
        rest[..end].trim_end().to_string()
    };
    Some((key.to_string(), value))
}

/// Set variables from a .env file without overriding ones that are already set (like python-dotenv).
fn load_env_file(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for (key, value) in text.trim_start_matches('\u{feff}').lines().filter_map(parse_env_line) {
        if std::env::var_os(&key).is_none() {
            std::env::set_var(key, value);
        }
    }
}

/// Load .env from the settings folder (without overriding real environment variables) and fix the folders.
pub fn init(home: PathBuf) -> &'static Paths {
    let home = resolve(&home);
    load_env_file(&home.join(".env"));
    let data = env_dir(&home, "SINGALONG_DATA_DIR", "data");
    let exports = env_dir(&home, "SINGALONG_EXPORTS_DIR", "exports");
    let paths = Paths {
        bin: home.join("bin"),
        songs: data.join("songs"),
        performances: data.join("performances"),
        presentations: exports.join("presentations"),
        videos: exports.join("videos"),
        home,
    };
    PATHS.get_or_init(|| paths)
}

/// Make sure the data and export folders exist (Python: init_db).
pub fn ensure_dirs() {
    let p = paths();
    for dir in [&p.songs, &p.performances, &p.presentations, &p.videos] {
        let _ = std::fs::create_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::parse_env_line;

    fn p(l: &str) -> Option<(String, String)> {
        parse_env_line(l)
    }

    #[test]
    fn env_lines_like_python_dotenv() {
        assert_eq!(p(r"SINGALONG_DATA_DIR=C:\Users\omerk\Downloads\Nurit\data"), Some(("SINGALONG_DATA_DIR".into(), r"C:\Users\omerk\Downloads\Nurit\data".into())));
        assert_eq!(p("A='x y'  # c"), Some(("A".into(), "x y".into())));
        assert_eq!(p(r#"A="a\nb" # c"#), Some(("A".into(), "a\nb".into())));
        assert_eq!(p("export A=1 # c"), Some(("A".into(), "1".into())));
        assert_eq!(p("A="), Some(("A".into(), String::new())));
        assert_eq!(p("# A=1"), None);
        assert_eq!(p("A=b#notcomment"), Some(("A".into(), "b#notcomment".into())));
    }
}
