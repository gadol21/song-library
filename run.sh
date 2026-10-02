#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

# Check if .venv exists, otherwise create it
if [ ! -d ".venv" ]; then
    echo "Creating Python virtual environment (.venv)..."
    python3 -m venv .venv
    .venv/bin/pip install --upgrade pip
    .venv/bin/pip install -r requirements.txt
fi

echo "=================================================="
echo "  🎤 Sing-Along Studio | אולפן שירה בציבור וקריוקי"
echo "  Starting server at http://localhost:8000"
echo "=================================================="

# Run uvicorn
exec .venv/bin/uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
