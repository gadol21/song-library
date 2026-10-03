#!/usr/bin/env bash
# Web server mode (http://localhost:8000). The desktop app on Windows is just SingAlongStudio.exe.
cd "$(dirname "${BASH_SOURCE[0]}")"
exec cargo run --release -- --server --port 8000
