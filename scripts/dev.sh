#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT/dev/windie-inspector"
TOKEN_FILE="${WINDIE_DEV_TOKEN_FILE:-$ROOT/target/windie-dev-api-token}"

if [ ! -d "$UI/node_modules" ]; then
  echo "Frontend dependencies are missing. Run scripts/setup.sh first." >&2
  exit 1
fi

export WINDIE_DATA_DIR="${WINDIE_DEV_DATA_DIR:-$ROOT/target/windie-dev-data}"
export WINDIE_CONFIG_DIR="${WINDIE_DEV_CONFIG_DIR:-$ROOT/target/windie-dev-config}"

mkdir -p "$(dirname "$TOKEN_FILE")"
if [ -z "${WINDIE_API_TOKEN:-}" ]; then
  if [ ! -s "$TOKEN_FILE" ]; then
    umask 077
    od -An -N16 -tx1 /dev/urandom | tr -d ' \n' > "$TOKEN_FILE"
  fi
  export WINDIE_API_TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
else
  umask 077
  printf '%s\n' "$WINDIE_API_TOKEN" > "$TOKEN_FILE"
fi

export VITE_WINDIE_API_URL="${VITE_WINDIE_API_URL:-http://127.0.0.1:8787}"
export VITE_WINDIE_API_TOKEN="$WINDIE_API_TOKEN"

api_pid=""
ui_pid=""

cleanup() {
  trap - EXIT INT TERM HUP
  if [ -n "$api_pid" ]; then
    kill "$api_pid" 2>/dev/null || true
  fi
  if [ -n "$ui_pid" ]; then
    kill "$ui_pid" 2>/dev/null || true
  fi
  if [ -n "$api_pid" ]; then
    wait "$api_pid" 2>/dev/null || true
  fi
  if [ -n "$ui_pid" ]; then
    wait "$ui_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT INT TERM HUP

echo "Windie development data: $WINDIE_DATA_DIR"
echo "Windie development UI: http://localhost:3000/?windie_token=$WINDIE_API_TOKEN"
echo "Rust changes require restarting scripts/dev.sh; frontend changes hot reload."

(
  cd "$ROOT"
  exec cargo run --manifest-path "$ROOT/Cargo.toml" -- api
) &
api_pid=$!

(
  cd "$UI"
  exec npm run start
) &
ui_pid=$!

while kill -0 "$api_pid" 2>/dev/null && kill -0 "$ui_pid" 2>/dev/null; do
  sleep 1
done

set +e
if ! kill -0 "$api_pid" 2>/dev/null; then
  wait "$api_pid"
  exit_code=$?
else
  wait "$ui_pid"
  exit_code=$?
fi
set -e

exit "$exit_code"
