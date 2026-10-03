"""Desktop entry point: runs the Sing-Along Studio server inside a native window (pywebview / Edge WebView2).

Website mode is unaffected: it still starts with `uvicorn app.main:app` (run.bat / run.sh). Everything that
must happen before `app.*` is imported (settings folder, log file, PATH) is done at the top of this file.
"""
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

APP_NAME = "Sing-Along Studio"

# Where bundled files live: next to this file when run from source, in PyInstaller's folder when frozen
RESOURCE_DIR = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))


def prepare_environment() -> Path:
    """Settings folder, log file and PATH. Must run before app.database is imported."""
    home = Path(os.environ.get("SINGALONG_HOME") or Path(os.environ.get("APPDATA", Path.home())) / APP_NAME)
    home.mkdir(parents=True, exist_ok=True)
    os.environ["SINGALONG_HOME"] = str(home)

    # First run: give the user a .env to edit (Gemini key, data/exports folders)
    env_file, example = home / ".env", RESOURCE_DIR / ".env.example"
    if not env_file.exists() and example.exists():
        shutil.copyfile(example, env_file)

    # A windowed exe has no console, and uvicorn's logging fails on a missing stdout/stderr
    if sys.stdout is None or sys.stderr is None:
        logs = home / "logs"
        logs.mkdir(exist_ok=True)
        log = open(logs / "app.log", "a", encoding="utf-8", buffering=1)
        sys.stdout = sys.stdout or log
        sys.stderr = sys.stderr or log

    # Same as run.bat: the bundled FFmpeg goes ahead of any system install
    ffmpeg_bin = RESOURCE_DIR / "ffmpeg" / "bin"
    if ffmpeg_bin.is_dir():
        os.environ["PATH"] = f"{ffmpeg_bin}{os.pathsep}{os.environ.get('PATH', '')}"
    return home


def hide_child_consoles() -> None:
    """Without this every ffmpeg / ffprobe / yt-dlp child process flashes a console window."""
    if os.name != "nt":
        return
    real_popen_init = subprocess.Popen.__init__

    def popen_init(self, *args, **kwargs):
        kwargs["creationflags"] = kwargs.get("creationflags", 0) | subprocess.CREATE_NO_WINDOW
        real_popen_init(self, *args, **kwargs)

    subprocess.Popen.__init__ = popen_init


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class DesktopApi:
    """Methods the page can call as window.pywebview.api.<name>()."""

    def __init__(self):
        self._window = None

    def save_export(self, folder: str, filename: str):
        """Copy a finished export to a place the user picks. Returns the saved path, or None if cancelled."""
        import webview
        from app.database import PRESENTATIONS_DIR, VIDEOS_DIR

        source_dir = {"presentations": PRESENTATIONS_DIR, "videos": VIDEOS_DIR}.get(folder)
        if source_dir is None or Path(filename).name != filename:
            raise ValueError("Invalid export")
        source = source_dir / filename
        if not source.is_file():
            raise FileNotFoundError(filename)
        chosen = self._window.create_file_dialog(webview.SAVE_DIALOG, save_filename=filename)
        if not chosen:
            return None
        target = chosen if isinstance(chosen, str) else chosen[0]
        shutil.copyfile(source, target)
        return str(target)


def main() -> None:
    prepare_environment()
    hide_child_consoles()

    import uvicorn
    import webview
    from app.main import app

    port = free_port()
    server = uvicorn.Server(uvicorn.Config(app, host="127.0.0.1", port=port, log_level="info"))
    threading.Thread(target=server.run, daemon=True).start()
    while not server.started:
        time.sleep(0.05)

    api = DesktopApi()
    api._window = webview.create_window(
        f"{APP_NAME} | אולפן שירה בציבור וקריוקי", f"http://127.0.0.1:{port}/",
        js_api=api, width=1400, height=900, min_size=(900, 600))
    webview.start()

    server.should_exit = True


if __name__ == "__main__":
    main()
