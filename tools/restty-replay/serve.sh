#!/usr/bin/env bash
# Serve the replay page on http://localhost:${PORT:-8765}/ (run record.sh and npm install first).
set -euo pipefail
cd "$(dirname "$0")"
[ -f node_modules/restty/dist/restty.esm.js ] || npm install --silent
[ -d streams ] || { echo "no streams: run ./record.sh first" >&2; exit 1; }
mkdir -p fonts
font=$(fc-match -f '%{file}' 'JetBrains Mono:style=Regular' 2>/dev/null || true)
[ -f fonts/mono.ttf ] || { [ -n "$font" ] && cp "$font" fonts/mono.ttf; } || true
exec python3 -m http.server "${PORT:-8765}" --bind 127.0.0.1
