@echo off
REM Builds the portable desktop app: dist\SingAlongStudio.exe (one file, no installer, no other files needed)
cd /d "%~dp0"
cargo build --release
if errorlevel 1 exit /b 1
if not exist dist mkdir dist
copy /y target\release\SingAlongStudio.exe dist\SingAlongStudio.exe
echo.
echo Done: dist\SingAlongStudio.exe
