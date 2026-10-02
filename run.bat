@echo off
setlocal

REM Use UTF-8 so the Hebrew banner displays correctly
chcp 65001 >nul

cd /d "%~dp0"

REM Use the bundled FFmpeg (ffmpeg\bin) ahead of any system install
if not exist "ffmpeg\bin\ffmpeg.exe" (
    echo Bundled FFmpeg not found at ffmpeg\bin\ffmpeg.exe
    pause
    exit /b 1
)
set "PATH=%~dp0ffmpeg\bin;%PATH%"

REM Check if .venv exists, otherwise create it
if not exist ".venv\" (
    echo Creating Python virtual environment ^(.venv^)...
    call :make_venv
    if errorlevel 1 goto :error
    ".venv\Scripts\python.exe" -m pip install --upgrade pip
    if errorlevel 1 goto :error
    ".venv\Scripts\python.exe" -m pip install -r requirements.txt
    if errorlevel 1 goto :error
)

echo ==================================================
echo   🎤 Sing-Along Studio ^| אולפן שירה בציבור וקריוקי
echo   Starting server at http://localhost:8000
echo ==================================================

REM Run uvicorn
".venv\Scripts\python.exe" -m uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
goto :eof

:make_venv
REM Prefer the Python launcher (py) if present, fall back to python
where py >nul 2>nul
if errorlevel 1 (
    python -m venv .venv
) else (
    py -3 -m venv .venv
)
exit /b %errorlevel%

:error
echo.
echo Setup failed. Make sure Python 3 is installed and on your PATH.
if exist ".venv\" rmdir /s /q ".venv"
pause
exit /b 1
