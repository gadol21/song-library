# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Sing-Along Studio: a Rust (axum) server plus a no-build vanilla-JS web UI (UI text is Hebrew/RTL) for managing songs, timing lyrics verse by verse, and exporting PowerPoint decks and MP4 karaoke videos for singalong events. See `README.md` for the feature tour.

## Commands

- Run (web server on :8000): `run.bat` / `./run.sh` = `cargo run --release -- --server --port 8000`. Without `--server` the exe opens the desktop window (Windows). A test server on another port: `cargo run --release -- --server --host 127.0.0.1 --port 8801`, with `SINGALONG_HOME` set to a scratch folder.
- Build the portable exe: `build_desktop.bat` (-> `dist\SingAlongStudio.exe`, one file; the C runtime is linked in, so no redistributables). The UI (`app/static`) is embedded in release builds (debug builds read it from disk).
- Tests: `cargo test --lib` (unit tests that pin behavior to the old Python version: Pillow text widths, json formatting, rounding, prompts/parsing).
- There is no linter. Verify changes by running the server and exercising the UI/API (Playwright works well).
- Settings come from `.env` in the settings folder (copy `.env.example`; gitignored; restart after editing): `SINGALONG_DATA_DIR`, `SINGALONG_EXPORTS_DIR`, `GOOGLE_AI_API_KEY`, `GEMINI_THINKING_LEVEL`. Server mode uses the current folder as the settings folder; the desktop app uses `%APPDATA%\Sing-Along Studio`. `SINGALONG_HOME` overrides both (always set it to a scratch folder when testing the desktop app).

## History: this was a Python app

The backend was ported from FastAPI/Python to Rust to get a single small portable exe. The port keeps the Python version's behavior on purpose, especially everything the AI sees: prompts, response schemas, request JSON (serialized like Python's `json.dumps`), upload protocol, page-text extraction (a port of `html.parser`), mime types, thresholds. When touching these, compare against the old implementation (`git show <commit before the port>:app/...`). Things deliberately reproduced:
- Font sizes use Pillow's exact text measuring (hinted advances, quirky kerning) on Arial Bold: `text_fit.rs`.
- Stored JSON/LRC files are written like Python wrote them (`py.rs`: `float_repr`, indent=2, CRLF on Windows). PPTX parts are byte-identical to python-pptx's (templates in `assets/pptx`).
- The video is drawn by our own code to match libass pixel for pixel (mean difference ~0.7/255 on a test frame). Notably libass laid Hebrew out in *logical order left to right with no bidi reordering*, and `video.rs` reproduces that. VFR timing snaps to 1/25 s like FFmpeg's concat demuxer did.

## Architecture

**Storage is the filesystem, not a database** (`storage.rs`). `DATA_DIR/songs/<id>/` holds `song.json` (metadata + `verses[]` with `start_time`/`end_time`), `audio.*` and a generated `lyrics.lrc`; `DATA_DIR/performances/<id>.json` holds a setlist (`song_ids`) plus its colors (`bg_color`, `text_color`, `title_color`). Ids are slugs of the title. Songs are `serde_json` objects in key order, so new fields (e.g. `youtube_url`) persist with no backend change. Two sample songs are seeded when the library is empty (`sample.rs`).

**Exports share one input shape.** `/api/export/pptx` and `/api/export/video` load a song or performance, resolve colors via `theme.rs`, and call `pptx::generate_presentation` / `video::generate_karaoke_video` / `generate_performance_video`. `text_fit.rs` picks one font size per song (the largest that fits its longest verse).

**PPTX audio** (`pptx.rs`): `wait_for_click` (default on, set by the export dialog) makes each song with audio stay on its title slide until the presenter clicks, then the audio and timed slides start (when the audio starts on the title slide, an identical copy of the title carries the audio and intro time). A song with audio gets it embedded (audio/media relationships, a hidden picture, a `p:timing` tree with `numSld` so it plays across the song's slides). Timed songs also get `advTm` auto-advance per slide. The audio starts on the title slide, or on verse 1 when it begins before 2 s. Non-PowerPoint audio formats are converted to MP3 (LAME). Not opened in real PowerPoint during development: only the XML structure was checked.

**Video pipeline** (`video.rs`, `mp4.rs`, `audio.rs`), no FFmpeg:
- Layout is described as an ASS script on a 1280x720 canvas (built exactly as before, `generate_ass_subtitles`); our renderer draws it at 1920x1080 (`OUTPUT_W/H`) with rustybuzz + ttf-parser outlines + tiny-skia, then OpenH264 (BSD, built from source; Cisco's patent grant applies only to Cisco's own binary, the owner accepted that) encodes H.264 and FDK-AAC encodes audio. `mp4.rs` is a small MP4 reader/writer (fast-start, edit list for the AAC priming).
- Each distinct screen is drawn once. A frame rate gives constant frame rate (default `DEFAULT_FRAME_RATE`; every change snapped to the frame grid); `null` gives variable frame rate (one frame per screen, 90 kHz timescale, changes on a 1/25 s grid and one extra tick at the end, like the old FFmpeg output).
- Performance videos are encoded as one continuous movie (the old version rendered each song and joined them).
- Arial Bold and Arial must exist on the machine (Windows has them; Linux uses Liberation Sans). If the measuring font is missing, `text_fit.rs` falls back to estimates.
- Audio lengths (`audio::probe_duration`) are computed like ffprobe reported them (MP3 Xing/LAME tag math, MP4 edit lists) because they appear in AI prompts.

**YouTube** (`youtube.rs`): yt-dlp and QuickJS-ng are downloaded on first use into `<home>/bin` (hash-verified; QuickJS pinned, yt-dlp checked against its published SHA2-256SUMS), updated with `yt-dlp -U` weekly and on the first refused download. yt-dlp runs with `--no-js-runtimes --js-runtimes quickjs:<path>` and a nonexistent `--ffmpeg-location` so no system FFmpeg ever takes part. Audio is requested as m4a (AAC) and converted to MP3 192k here; videos are downloaded as separate streams and joined by `mp4::merge_streams`. Retries go through alternate YouTube player clients. The link is remembered as `studioYoutubeUrl` and saved as `youtube_url`.

**AI lyrics search** (`gemini_lyrics.rs`, `POST /api/ai/find-lyrics`): three steps: (1) Gemini with Google Search is asked for the *titles* of pages with the lyrics, and the page links come from the answer's grounding metadata (asking for site names let it answer from memory without searching); (2) the app downloads those pages and turns them into plain text generically (`html_text.rs`: charset from `<meta>` for old windows-1255 Hebrew sites, browser User-Agent, YouTube skipped); (3) Gemini without tools copies the lyrics out of the best page (JSON: page number + lyrics). Don't go back to asking Gemini for the lyrics directly: its copyright filter (RECITATION) blocked about 4 in 5 such answers while copying from supplied pages was never blocked. The user prefers approaches that work consistently over retries. Cost includes the grounding fee (list price shown).

**AI timing** (`gemini_timing.rs`, `gemini.rs`, model `gemini-3.8-flash`): two studio menu options share `run_gemini` (upload, ask, parse, cost, delete uploads): time verses from the song's own YouTube video, or from the song's audio (the karaoke version) guided by a YouTube version with a singer. The app downloads and uploads the media itself (never send links to Gemini). Prompts ask for `MM:SS.s` timestamps because plain seconds drifted outside the song. `parse_timings` drops times that are missing, out of range or not increasing rather than guessing. Cost uses the price schedule (price doubles on 2027-01-01). Thinking level defaults to low. `gemini.rs` talks REST directly and sends what google-genai sent (same JSON bytes, resumable upload in 8 MB chunks, same mime table). The API key is the user's real paid key: keep test calls few and never print it. To test without spending: point `GOOGLE_GEMINI_BASE_URL` at a stand-in server.

**Desktop app** (`desktop.rs`): the icon (red microphone in front of singing lips) is drawn by `tools/make_icon.py` (needs Pillow) into `assets/icon.ico` + `icon.png`; `build.rs` embeds the .ico in the exe (winresource) and `desktop.rs` uses the PNG for the window. wry/tao (Edge WebView2) opens a window on the same axum app, served on a free `127.0.0.1` port in a thread. WebView2 does not download files, so a small injected script provides `window.pywebview.api.save_export` (name kept from the Python app so `app.js` is unchanged); it calls the desktop-only route `/__desktop/save_export`, which opens a native Save As dialog (rfd) and copies the export. The first run writes `.env` from `.env.example`; with no console, logs go to `<home>/logs/app.log`. WebView2's files live in `<home>/webview`.

**Frontend** is one file, `app/static/js/app.js` (tabs: library, studio, setlist, plus a fullscreen live stage). The studio's lyrics box is the source of truth: `syncVersesFromLyrics` re-splits on blank lines and aligns old and new verses by text similarity so editing never discards timings of unchanged verses. `/` serves `index.html` with `?v=<mtime>` added to the script/style URLs and static files send `Cache-Control: no-cache`, so updates show without a hard refresh.

## Gotchas

- A dev server is often already running on :8000 and shares `data/` and `exports/` with any test server you start. Use another port and a scratch `SINGALONG_HOME`, and delete only files you created; `exports/videos` contains the user's own exports.
- Frame times from ffprobe/FFmpeg (if you install one to inspect output; the app never uses it) for sparse (variable-frame-rate) video are easy to misread: `-ss` returns the first frame at or after the time, not the one on screen. Extract all frames with their times (`showinfo`) when checking them.
- `AacEncoder` wraps FDK-AAC by hand (the `fdk-aac` crate can't flush); never build a second value from a first one with `..` struct update, its `Drop` closes the handle.
