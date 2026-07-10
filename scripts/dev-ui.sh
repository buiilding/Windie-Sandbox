#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT/dev/windie-inspector"

cd "$UI"
if [ ! -d node_modules ]; then
  npm install --legacy-peer-deps
fi

export REACT_APP_WINDIE_API_URL="${REACT_APP_WINDIE_API_URL:-http://127.0.0.1:8787}"
exec npm run start

