import os
import json
import tempfile
from pathlib import Path
from typing import Optional, List

from fastapi import FastAPI, UploadFile, File, Form, HTTPException, Query
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.cors import CORSMiddleware
from fastapi.concurrency import run_in_threadpool
from pydantic import BaseModel
from urllib.parse import quote

from app.database import (
    init_db, list_songs, get_song, save_song, delete_song,
    list_performances, get_performance, save_performance, delete_performance,
    SONGS_DIR, PRESENTATIONS_DIR, VIDEOS_DIR
)
from app.pptx_generator import generate_presentation
from app.video_generator import generate_karaoke_video, generate_performance_video, check_frame_rate, DEFAULT_FRAME_RATE
from app.sample_data import seed_sample_data_if_empty
from app.theme import resolve_theme
from app.youtube import download_audio_mp3, AudioDownloadError
from app.gemini_timing import time_verses_from_youtube, time_verses_from_singer_version, GeminiTimingError

app = FastAPI(title="Sing-Along Studio API", version="1.0.0")

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

BASE_DIR = Path(__file__).resolve().parent
STATIC_DIR = BASE_DIR / "static"

@app.on_event("startup")
def startup_event():
    init_db()
    seed_sample_data_if_empty()

# ----------------- SONGS API -----------------

@app.get("/api/songs")
def api_list_songs(search: Optional[str] = None, language: Optional[str] = None):
    return list_songs(search=search, language=language)

@app.get("/api/songs/{song_id}")
def api_get_song(song_id: str):
    song = get_song(song_id)
    if not song:
        raise HTTPException(status_code=404, detail="Song not found")
    return song

@app.post("/api/songs")
async def api_save_song(
    song_json: str = Form(...),
    audio_file: Optional[UploadFile] = File(None)
):
    try:
        data = json.loads(song_json)
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Invalid JSON data: {e}")

    audio_bytes = None
    audio_ext = ".mp3"
    if audio_file and audio_file.filename:
        audio_bytes = await audio_file.read()
        audio_ext = Path(audio_file.filename).suffix or ".mp3"

    saved = save_song(data, audio_bytes=audio_bytes, audio_ext=audio_ext)
    return saved

@app.delete("/api/songs/{song_id}")
def api_delete_song(song_id: str):
    success = delete_song(song_id)
    if not success:
        raise HTTPException(status_code=404, detail="Song not found")
    return {"status": "deleted", "id": song_id}

@app.get("/api/songs/{song_id}/audio")
def api_get_song_audio(song_id: str):
    song = get_song(song_id)
    if not song or not song.get("audio_path"):
        raise HTTPException(status_code=404, detail="Audio file not found")
    path = Path(song["audio_path"])
    if not path.exists():
        raise HTTPException(status_code=404, detail="Audio file missing on disk")
    return FileResponse(path, media_type="audio/mpeg", filename=path.name)

@app.get("/api/songs/{song_id}/lrc")
def api_get_song_lrc(song_id: str):
    folder = SONGS_DIR / song_id
    lrc_file = folder / "lyrics.lrc"
    if not lrc_file.exists():
        raise HTTPException(status_code=404, detail="LRC file not found")
    return FileResponse(lrc_file, media_type="text/plain", filename=f"{song_id}.lrc")

class AudioFromUrlRequest(BaseModel):
    url: str

@app.post("/api/audio/from-url")
def api_audio_from_url(req: AudioFromUrlRequest):
    """Download a video link's audio as MP3. The studio then treats it like an uploaded file."""
    try:
        mp3_bytes, info = download_audio_mp3(req.url)
    except AudioDownloadError as e:
        raise HTTPException(status_code=400, detail=str(e))
    # Titles may be Hebrew, so send metadata percent-encoded in headers
    return Response(content=mp3_bytes, media_type="audio/mpeg", headers={
        "X-Audio-Title": quote(info["title"]),
        "X-Audio-Artist": quote(info["artist"]),
    })

class AiTimingRequest(BaseModel):
    youtube_url: str
    verses: List[str]
    title: str = ""
    artist: str = ""

@app.post("/api/ai/time-verses")
def api_ai_time_verses(req: AiTimingRequest):
    """Time the verses against the song's YouTube video with Gemini (takes a minute or so)."""
    try:
        return time_verses_from_youtube(req.youtube_url, req.verses, title=req.title, artist=req.artist)
    except GeminiTimingError as e:
        raise HTTPException(status_code=400, detail=str(e))

@app.post("/api/ai/time-verses-singer")
async def api_ai_time_verses_singer(
    singer_url: str = Form(...),
    verses: str = Form(...),
    title: str = Form(""),
    artist: str = Form(""),
    song_id: Optional[str] = Form(None),
    karaoke_audio: Optional[UploadFile] = File(None),
):
    """Time the verses in the song's own audio (the karaoke version) with the help of a version with a singer.

    The karaoke audio is the uploaded file, or else the audio saved for song_id.
    """
    try:
        verse_texts = json.loads(verses)
        if not isinstance(verse_texts, list) or not all(isinstance(v, str) for v in verse_texts):
            raise ValueError
    except ValueError:
        raise HTTPException(status_code=400, detail="רשימת הבתים אינה תקינה")

    with tempfile.TemporaryDirectory() as tmp:
        if karaoke_audio is not None and karaoke_audio.filename:
            karaoke_path = Path(tmp) / f"karaoke{Path(karaoke_audio.filename).suffix or '.mp3'}"
            karaoke_path.write_bytes(await karaoke_audio.read())
        else:
            song = get_song(song_id) if song_id else None
            if not song or not song.get("audio_path"):
                raise HTTPException(status_code=400, detail="לשיר אין קובץ שמע. הוסף קובץ שמע או שמור את השיר קודם.")
            karaoke_path = Path(song["audio_path"])
        try:
            # Downloading, uploading and waiting for Gemini block, so keep them off the event loop
            return await run_in_threadpool(
                time_verses_from_singer_version, singer_url, karaoke_path, verse_texts, title, artist)
        except GeminiTimingError as e:
            raise HTTPException(status_code=400, detail=str(e))

# ----------------- PERFORMANCES API -----------------

@app.get("/api/performances")
def api_list_performances():
    return list_performances()

@app.get("/api/performances/{perf_id}")
def api_get_performance(perf_id: str):
    perf = get_performance(perf_id)
    if not perf:
        raise HTTPException(status_code=404, detail="Performance not found")
    return perf

@app.post("/api/performances")
def api_save_performance(perf_data: dict):
    saved = save_performance(perf_data)
    return saved

@app.delete("/api/performances/{perf_id}")
def api_delete_performance(perf_id: str):
    success = delete_performance(perf_id)
    if not success:
        raise HTTPException(status_code=404, detail="Performance not found")
    return {"status": "deleted", "id": perf_id}

# ----------------- EXPORT API -----------------

class ExportPptxRequest(BaseModel):
    performance_id: Optional[str] = None
    song_id: Optional[str] = None

@app.post("/api/export/pptx")
def api_export_pptx(req: ExportPptxRequest):
    songs_to_export = []
    title = "שירה בציבור"
    prefix = "presentation"
    theme = None

    if req.performance_id:
        perf = get_performance(req.performance_id)
        if not perf:
            raise HTTPException(status_code=404, detail="Performance not found")
        songs_to_export = perf.get("songs", [])
        title = perf.get("title", "שירה בציבור")
        prefix = perf.get("id", "performance")
        theme = resolve_theme(perf)
    elif req.song_id:
        song = get_song(req.song_id)
        if not song:
            raise HTTPException(status_code=404, detail="Song not found")
        songs_to_export = [song]
        title = song.get("title", "")
        prefix = song.get("id", "song")
    else:
        raise HTTPException(status_code=400, detail="Provide either performance_id or song_id")

    if not songs_to_export:
        raise HTTPException(status_code=400, detail="No songs to export")

    file_path = generate_presentation(songs_to_export, title=title, filename_prefix=prefix, theme=theme)
    filename = Path(file_path).name

    return {
        "success": True,
        "filename": filename,
        "download_url": f"/api/downloads/presentations/{filename}",
        "song_count": len(songs_to_export)
    }

class ExportVideoRequest(BaseModel):
    song_id: Optional[str] = None
    performance_id: Optional[str] = None
    # A number = constant frame rate in frames per second (plays everywhere). An explicit null = variable frame rate
    # (fastest, but plays in VLC only). When the field is left out, a constant rate is used.
    frame_rate: Optional[float] = None

@app.post("/api/export/video")
def api_export_video(req: ExportVideoRequest):
    try:
        frame_rate = check_frame_rate(req.frame_rate if "frame_rate" in req.model_fields_set else DEFAULT_FRAME_RATE)
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    if req.performance_id:
        perf = get_performance(req.performance_id)
        if not perf:
            raise HTTPException(status_code=404, detail="Performance not found")
        songs = perf.get("songs", [])
        if not songs:
            raise HTTPException(status_code=400, detail="Performance contains no songs")
        file_path = generate_performance_video(songs, title=perf.get("title", "הופעה"), filename_prefix=perf.get("id", "performance"),
                                               theme=resolve_theme(perf), frame_rate=frame_rate)
        filename = Path(file_path).name
        return {
            "success": True,
            "filename": filename,
            "download_url": f"/api/downloads/videos/{filename}",
            "title": perf.get("title"),
            "song_count": len(songs)
        }
    elif req.song_id:
        song = get_song(req.song_id)
        if not song:
            raise HTTPException(status_code=404, detail="Song not found")

        file_path = generate_karaoke_video(song, filename_prefix=song.get("id", "karaoke"), frame_rate=frame_rate)
        filename = Path(file_path).name

        return {
            "success": True,
            "filename": filename,
            "download_url": f"/api/downloads/videos/{filename}",
            "song_title": song.get("title")
        }
    else:
        raise HTTPException(status_code=400, detail="Provide either song_id or performance_id")

@app.get("/api/downloads/{folder}/{filename}")
def api_download_file(folder: str, filename: str):
    if folder == "presentations":
        target = PRESENTATIONS_DIR / filename
        media_type = "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    elif folder == "videos":
        target = VIDEOS_DIR / filename
        media_type = "video/mp4"
    else:
        raise HTTPException(status_code=400, detail="Invalid folder")

    if not target.exists():
        raise HTTPException(status_code=404, detail="File not found")

    return FileResponse(target, media_type=media_type, filename=filename)

# ----------------- STATIC UI -----------------

class RevalidatedStaticFiles(StaticFiles):
    """Static files that the browser re-checks on every load (cheap: it gets "not modified" when nothing changed).

    Without this, browsers keep using their stored copy of the page's script and styles for hours, so an
    update to the app would not show up until the user hard-refreshes.
    """
    async def get_response(self, path, scope):
        response = await super().get_response(path, scope)
        response.headers["Cache-Control"] = "no-cache"
        return response

app.mount("/static", RevalidatedStaticFiles(directory=str(STATIC_DIR)), name="static")

@app.get("/")
def index():
    html = (STATIC_DIR / "index.html").read_text(encoding="utf-8")
    # Put each asset's last-modified time in its address, so a changed file is always fetched fresh
    for asset in ("css/style.css", "js/app.js"):
        version = int((STATIC_DIR / asset).stat().st_mtime)
        html = html.replace(f"/static/{asset}", f"/static/{asset}?v={version}")
    return HTMLResponse(html, headers={"Cache-Control": "no-cache"})
