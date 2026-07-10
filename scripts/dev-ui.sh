#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT/dev/windie-inspector"
TOKEN_FILE="${WINDIE_DEV_TOKEN_FILE:-$ROOT/target/windie-dev-api-token}"

if [ -n "${WINDIE_API_TOKEN:-}" ]; then
  API_TOKEN="$WINDIE_API_TOKEN"
elif [ -s "$TOKEN_FILE" ]; then
  API_TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
else
  echo "Windie development API token not found." >&2
  echo "Start scripts/dev-api.sh before scripts/dev-ui.sh." >&2
  exit 1
fi

cd "$UI"
if [ ! -d node_modules ]; then
  npm install --legacy-peer-deps
fi

export REACT_APP_WINDIE_API_URL="${REACT_APP_WINDIE_API_URL:-http://127.0.0.1:8787}"
export BROWSER=none
UI_URL="http://localhost:3000/?windie_token=$API_TOKEN"
echo "Windie development UI: $UI_URL"

open_ui_when_ready() {
  attempts=0
  while [ "$attempts" -lt 120 ]; do
    if curl --fail --silent --output /dev/null http://localhost:3000/; then
      if command -v open >/dev/null 2>&1; then
        open "$UI_URL"
      elif command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$UI_URL" >/dev/null 2>&1
      fi
      return
    fi
    attempts=$((attempts + 1))
    sleep 0.25
  done
}

open_ui_when_ready &
exec npm run start
