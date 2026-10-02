# 🎤 Sing-Along Studio | אולפן שירה בציבור וקריוקי

A streamlined, non-technical studio designed for sing-along singers, community singing hosts (*Shira BeTzibur*), and choir leaders. Easily manage your song collection, sync lyrics verse-by-verse, and generate stage-ready PowerPoint presentations or MP4 karaoke videos with one click.

---

## ✨ Key Features

1. **📁 Transparent Filesystem "Database"**:
   - Everything is stored in human-readable files under `data/songs/<song-name>/`.
   - Each song has `audio.mp3`, `song.json` (metadata & verse timestamps), and standard `lyrics.lrc`.
   - Easy to back up to Google Drive, Dropbox, or a USB stick with zero database setup.

2. **🎙️ "Tap-to-Sync" Studio (Verse-by-Verse Timing)**:
   - Paste lyrics and split into verses/stanzas with one click.
   - Hit Play and simply press the **Spacebar** whenever the next verse begins.
   - Micro-adjust timings anytime with `[-0.5s]` and `[+0.5s]` buttons.
   - Real-time stage preview lets you verify the visual flow before saving.

3. **🇮🇱 Seamless Hebrew (RTL) & English Support**:
   - Automatic language & RTL detection.
   - Full bidirectional text support for PowerPoint and Video subtitles (proper placement of punctuation and parentheses).

4. **📊 1-Click PowerPoint (.pptx) Generator**:
   - 16:9 widescreen layout designed for stage projectors and venue TVs.
   - High-contrast stage theme (midnight slate background, crisp large text, golden accents).
   - Generates intro title slides and slide-per-verse with song progress indicators.

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
Run the one-click startup script:
```bash
./run.sh
```
Or run directly:
```bash
.venv/bin/uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
```

### 2. Open in your Browser
Navigate to:
```
http://localhost:8000
```

The app comes pre-loaded with sample Hebrew and English songs ("עוד לא תמו כל פלאייך" and "Stand By Me") so you can immediately test PowerPoint exports, video rendering, and live stage mode!

---

## 📂 Project Structure

```
Nurit/
├── app/
│   ├── main.py              # FastAPI server & REST API
│   ├── database.py          # Filesystem database & LRC generator
│   ├── pptx_generator.py    # 16:9 Stage PowerPoint generator (RTL Hebrew enabled)
│   ├── video_generator.py   # FFmpeg MP4 karaoke video renderer
│   ├── sample_data.py       # Pre-seeded demo songs
│   └── static/              # Clean web UI (HTML, CSS, JS)
├── data/
│   ├── songs/               # Master song library (folders per song)
│   └── performances/        # Saved setlists
├── exports/
│   ├── presentations/       # Generated .pptx decks
│   └── videos/              # Generated .mp4 karaoke videos
├── requirements.txt
├── run.sh                   # Startup launcher
└── README.md
```
