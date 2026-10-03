@echo off
setlocal

REM Builds the portable desktop app: dist\SingAlongStudio\ and dist\SingAlongStudio-win64.zip
cd /d "%~dp0"

if not exist "ffmpeg\bin\ffmpeg.exe" (
    echo Bundled FFmpeg not found at ffmpeg\bin\ffmpeg.exe
    exit /b 1
)

if not exist ".venv\" (
    echo Creating Python virtual environment ^(.venv^)...
    py -3 -m venv .venv
    if errorlevel 1 goto :error
)

".venv\Scripts\python.exe" -m pip install -r requirements-desktop.txt
if errorlevel 1 goto :error

".venv\Scripts\python.exe" -m PyInstaller --noconfirm desktop.spec
if errorlevel 1 goto :error

if exist "dist\SingAlongStudio-win64.zip" del "dist\SingAlongStudio-win64.zip"
powershell -NoProfile -Command "Compress-Archive -Path 'dist\SingAlongStudio' -DestinationPath 'dist\SingAlongStudio-win64.zip'"
if errorlevel 1 goto :error

echo.
echo Done: dist\SingAlongStudio-win64.zip
exit /b 0

:error
echo.
echo Build failed.
exit /b 1
