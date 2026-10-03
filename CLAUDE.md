# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Sing-Along Studio: a FastAPI app plus a no-build vanilla-JS web UI (UI text is Hebrew/RTL) for managing songs, timing lyrics verse by verse, and exporting PowerPoint decks and MP4 karaoke videos for singalong events. See `README.md` for the feature tour (its video section is outdated: videos are now 1080p).

## Commands

- Run: `run.bat` (Windows) or `./run.sh`, which start `uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload`. `run.bat` puts the bundled `ffmpeg\bin` first on `PATH`; the `.exe` files are Git LFS. The scripts create `.venv` and install `requirements.txt` only when `.venv` does not exist, so after changing `requirements.txt` run `.venv\Scripts\pip install -r requirements.txt` yourself.
- Running uvicorn by hand (e.g. a test server on another port): `.venv/Scripts/python.exe -m uvicorn app.main:app --port 8765`, with `ffmpeg/bin` on `PATH` or video/audio features fail.
- Desktop app: `build_desktop.bat` (PyInstaller onedir from `desktop.spec`, entry point `desktop.py`, then zips to `dist/SingAlongStudio-win64.zip`). Run it from source with `.venv/Scripts/python.exe desktop.py`. Always set `SINGALONG_HOME` to a scratch folder when testing, or it uses the real `%APPDATA%\Sing-Along Studio`.
- There is no test suite and no linter. Verify changes by running the server and exercising the UI/API (Playwright works well) or by calling the generator functions directly from Python.
- Settings come from `.env` (copy `.env.example`; gitignored; restart after editing): `SINGALONG_DATA_DIR`, `SINGALONG_EXPORTS_DIR`, `GOOGLE_AI_API_KEY`, `GEMINI_THINKING_LEVEL`. It is loaded in `app/database.py` at import time.

## Architecture

**Storage is the filesystem, not a database** (`app/database.py`). `DATA_DIR/songs/<id>/` holds `song.json` (metadata + `verses[]` with `start_time`/`end_time`), `audio.*` and a generated `lyrics.lrc`; `DATA_DIR/performances/<id>.json` holds a setlist (`song_ids`) plus its colors (`bg_color`, `text_color`, `title_color`). Ids are slugs of the title. `save_song` writes whatever dict it is given, so new song fields (e.g. `youtube_url`) persist with no backend change. Two sample songs are seeded when the library is empty.

**Exports share one input shape.** `/api/export/pptx` and `/api/export/video` load a song or performance, resolve the performance's colors via `theme.resolve_theme`, and call `pptx_generator.generate_presentation` / `video_generator.generate_karaoke_video` / `generate_performance_video`. `text_fit.py` picks one font size per song (the largest that fits its longest verse, measured with Pillow against Arial Bold), used by both generators.

**PPTX audio** (`pptx_generator.py`): a song with audio gets it embedded (hand-built XML: audio/media relationships, a hidden picture, a `p:timing` tree with `numSld` so it plays across the song's slides). Timed songs also get `advTm` auto-advance per slide (verse start to next verse start). The audio starts on the title slide, or on verse 1 when it begins before 2 s. Not opened in real PowerPoint during development: only the XML structure was checked.

**Video pipeline** (`app/video_generator.py`) is the least obvious part:
- Layout is designed on a 1280x720 canvas (`VIDEO_W/H`, also the ASS `PlayRes`); output is 1920x1080 (`OUTPUT_W/H`) and libass scales the canvas. Change resolution with the OUTPUT constants only.
- The picture changes only when a subtitle appears or disappears, so `plan_screens`/`render_screens` draw each distinct screen once as a PNG (by remapping the ASS script so screen *i* sits at second *i*), then FFmpeg builds the video from a concat list.
- `frame_rate=None` gives variable frame rate (fastest, plays in VLC but not Windows Media Player; uses `-bf 0` and stops at duration + `END_PADDING` so track lengths are exact). A number gives constant frame rate (works everywhere); change times are snapped to the frame grid in Python because the concat demuxer counts image time in 1/25 s ticks. The API default when `frame_rate` is omitted is `DEFAULT_FRAME_RATE`; `null` must be sent explicitly for variable. The UI asks via `askVideoOptions()` before every export.
- Performance videos render each song, then join them with `-c copy`.

**YouTube** (`app/youtube.py`, yt-dlp): audio is converted to MP3 and returned to the browser, which treats it like a picked file; the link is remembered as `studioYoutubeUrl` and saved as `youtube_url`. Downloads retry through alternate YouTube player clients because some clients get HTTP 403; installed JS runtimes (node/deno) are passed to yt-dlp.

**AI timing** (`app/gemini_timing.py`, model `gemini-3.8-flash`): two studio menu options share `_run_gemini` (upload, ask, parse, cost, delete uploads): time verses from the song's own YouTube video, or from the song's audio (the karaoke version) guided by a YouTube version with a singer. Prompts ask for `MM:SS.s` timestamps because plain seconds drifted outside the song. `parse_timings` drops times that are missing, out of range or not increasing rather than guessing. Cost uses `PRICE_SCHEDULE` (the price doubles on 2027-01-01) and the usage field `candidates_token_count`. Thinking level defaults to low (measured: no accuracy gain from more). The API key is the user's real paid key: keep test calls few and never print it.

**Desktop app** (`desktop.py`): pywebview (Edge WebView2) opens a window on the same FastAPI app, served by uvicorn on a free `127.0.0.1` port in a thread. `SINGALONG_HOME` (default `%APPDATA%\Sing-Along Studio`) is where `.env` is loaded from and what relative data paths resolve against (`BASE_DIR` in `app/database.py`); `PROJECT_DIR` is the code/bundled-files folder (ffmpeg). `DATA_DIR`/`EXPORTS_DIR` stay configurable through `.env`. `desktop.py` must prepare the environment before importing `app.*`: log file when there is no console, bundled ffmpeg first on `PATH`, and a `subprocess.Popen` patch adding `CREATE_NO_WINDOW` so ffmpeg/yt-dlp don't flash consoles. WebView2 does not download files, so exports go through `deliverDownload()` in `app.js`, which calls `window.pywebview.api.save_export` (native Save As) when present and otherwise navigates to the download URL as the website does. Keep `DesktopApi` attributes private (`_window`): pywebview walks public attributes and recursed forever into the window object. HTML5 `requestFullscreen` already fills the screen in the window, so the live stage needs no desktop-specific code.

**Frontend** is one file, `app/static/js/app.js` (tabs: library, studio, setlist, plus a fullscreen live stage). The studio's lyrics box is the source of truth: `syncVersesFromLyrics` re-splits on blank lines and aligns old and new verses by text similarity so editing never discards timings of unchanged verses. `/` serves `index.html` with `?v=<mtime>` added to the script/style URLs and static files send `Cache-Control: no-cache`, so updates show without a hard refresh.

## Gotchas

- A dev server is often already running on :8000 and shares `data/` and `exports/` with any test server you start. Use another port, and delete only files you created (by exact name); `exports/videos` contains the user's own exports.
- FFmpeg filter paths need `escape_filter_path` (Windows drive colons). Subprocess output is decoded as UTF-8 with `errors="replace"`; printing Hebrew from ad-hoc Python on Windows needs `PYTHONIOENCODING=utf-8`.
- Frame times from FFmpeg for sparse (variable-frame-rate) video are easy to misread: `-ss` returns the first frame at or after the time, not the one on screen. Extract all frames with their times (`showinfo`) when checking them.
