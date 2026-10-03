//! Sing-Along Studio: song library, verse timing, PowerPoint decks and karaoke videos.

pub mod audio;
pub mod config;
#[cfg(windows)]
pub mod desktop;
pub mod gemini;
pub mod gemini_lyrics;
pub mod gemini_timing;
pub mod html_text;
pub mod log;
pub mod mp4;
pub mod pptx;
pub mod sample;
pub mod server;
pub mod py;
mod py_tables;
pub mod storage;
pub mod text_fit;
pub mod theme;
pub mod video;
pub mod youtube;
