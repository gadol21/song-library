@echo off
REM Web server mode (http://localhost:8000), like the old Python run.bat. The desktop app is just SingAlongStudio.exe.
chcp 65001 >nul
cd /d "%~dp0"
cargo run --release -- --server --port 8000
