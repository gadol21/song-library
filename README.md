# 🎤 Sing-Along Studio | אולפן שירה בציבור וקריוקי

A streamlined, non-technical studio designed for sing-along singers, community singing hosts (*Shira BeTzibur*), and choir leaders. Easily manage your song collection, sync lyrics verse-by-verse, and generate stage-ready PowerPoint presentations or MP4 karaoke videos with one click.

---

## ✨ Key Features

1. **📁 Transparent Filesystem "Database"**:
   - Everything is stored in human-readable files under `data/songs/<song-name>/`.
   - Each song has `audio.mp3`, `song.json` (metadata & verse timestamps), and standard `lyrics.lrc`.
   - Easy to back up to Google Drive, Dropbox, or a USB stick with zero database setup.
   - To keep the data somewhere else (e.g. a backed-up folder), copy `.env.example` to `.env` and set `SINGALONG_DATA_DIR` (songs + performances) and `SINGALONG_EXPORTS_DIR` (generated decks/videos). Restart the app after changing it.

2. **🎙️ "Tap-to-Sync" Studio (Verse-by-Verse Timing)**:
   - Paste lyrics and split into verses/stanzas with one click.
   - Hit Play and simply press the **Spacebar** whenever the next verse begins.
   - **🪄 Find lyrics (AI)**: with a title and artist filled in, Gemini searches Google and fills the lyrics box (the price of each request is shown). Needs a Gemini key in `.env`.
   - Micro-adjust timings anytime with `[-0.5s]` and `[+0.5s]` buttons.
   - Real-time stage preview lets you verify the visual flow before saving.

3. **🇮🇱 Seamless Hebrew (RTL) & English Support**:
   - Automatic language & RTL detection.
   - Full bidirectional text support for PowerPoint and Video subtitles (proper placement of punctuation and parentheses).

4. **📊 1-Click PowerPoint (.pptx) Generator**:
   - 16:9 widescreen layout designed for stage projectors and venue TVs.
   - High-contrast stage theme (midnight slate background, crisp large text, golden accents).
   - Generates intro title slides and slide-per-verse with song progress indicators.
   - Embeds the song's audio and, for timed songs, advances the slides by themselves in sync with it (start the slide show and it plays).

5. **🎬 1-Click Full Movie (.mp4) Generator**:
   - **Performance Movies**: Export the entire setlist into a single, seamless 720p HD MP4 video with intro cards, audio tracks, and synchronized lyrics!
   - **Song Movies**: Export individual karaoke video clips directly from the Song Library, Setlist Builder, or Studio.
   - Highlights the current verse in bold with outline, with upcoming verse preview.

6. **📋 Performance Setlist Builder & 🎤 Live Audio-Synced Stage**:
   - Select songs for an upcoming gig, reorder with `↑` / `↓`.
   - Export the entire show as a PowerPoint presentation or unified MP4 movie.
   - **Live Audio-Synced Stage View**: Runs directly in fullscreen in your browser! Plays the backing tracks automatically, synchronizes verse slides in real time with the music, and auto-transitions between songs in your setlist. Includes full manual override (Spacebar to pause, Arrow keys to jump).

---

## 🚀 Quick Start

### 1. Launch the Application
The backend is a single Rust program (needs the Rust toolchain to build from source). Run the web server:
```bash
./run.sh          # or run.bat on Windows; same as: cargo run --release -- --server --port 8000
```

### 2. Open in your Browser
Navigate to:
```
http://localhost:8000
```

The app comes pre-loaded with sample Hebrew and English songs ("עוד לא תמו כל פלאייך" and "Stand By Me") so you can immediately test PowerPoint exports, video rendering, and live stage mode!

---

## 🖥️ Desktop App (Windows, portable)

The same app also runs in its own window, as **one portable file** (no installer, no Python, no FFmpeg):

1. Double-click `SingAlongStudio.exe`. Windows SmartScreen warns about an unknown app on the first run (the exe is
   unsigned): choose **More info → Run anyway**. It needs the Edge WebView2 Runtime, which ships with Windows 11.
2. Settings live in `%APPDATA%\Sing-Along Studio\.env` (paste that into Explorer's address bar). Put your Gemini key there
   for the AI features, and optionally point `SINGALONG_DATA_DIR` / `SINGALONG_EXPORTS_DIR` at other folders
   (e.g. on a backed-up drive). Restart the app after editing. The defaults are `data` and `exports` inside that same folder.
3. Exports (PowerPoint / MP4) open a Save As dialog instead of downloading.
4. The first time you use a YouTube link, the app downloads yt-dlp and a small JavaScript runtime into
   `%APPDATA%\Sing-Along Studioin` (checked against checksums, kept up to date automatically).
5. If something goes wrong, `%APPDATA%\Sing-Along Studio\logspp.log` may help.

Build it with `build_desktop.bat` (needs the Rust toolchain); it writes `dist\SingAlongStudio.exe`.

---

## 📂 Project Structure

```
Nurit/
├── src/
│   ├── main.rs, desktop.rs   # Entry point; native window (WebView2) around the same server
│   ├── server.rs             # HTTP API and web UI (axum)
│   ├── storage.rs            # Filesystem database & LRC generator
│   ├── pptx.rs               # 16:9 PowerPoint generator (RTL Hebrew enabled)
│   ├── video.rs, mp4.rs      # Karaoke video: drawing, H.264/AAC encoding, MP4 writing
│   ├── audio.rs              # Decoding, MP3/AAC encoding, durations
│   ├── gemini*.rs            # Gemini REST client, AI timing, AI lyrics search
│   ├── youtube.rs            # yt-dlp (+ QuickJS) downloaded on first use
│   └── sample.rs             # Pre-seeded demo songs
├── app/static/               # Clean web UI (HTML, CSS, JS), embedded into the exe
├── assets/pptx/              # Fixed parts of the PowerPoint package
├── data/                     # Songs and performances (created at run time)
├── exports/                  # Generated .pptx decks and .mp4 videos
├── Cargo.toml
├── build_desktop.bat         # Builds dist\SingAlongStudio.exe
├── run.bat / run.sh          # Web server mode
└── README.md
```
