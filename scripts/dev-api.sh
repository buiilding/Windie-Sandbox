#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export WINDIE_DATA_DIR="${WINDIE_DEV_DATA_DIR:-$ROOT/target/windie-dev-data}"
export WINDIE_CONFIG_DIR="${WINDIE_DEV_CONFIG_DIR:-$ROOT/target/windie-dev-config}"

TOKEN_FILE="${WINDIE_DEV_TOKEN_FILE:-$ROOT/target/windie-dev-api-token}"
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

echo "Windie development data: $WINDIE_DATA_DIR"
echo "The installed Windie runtime and user database are not modified."
exec cargo run --manifest-path "$ROOT/Cargo.toml" -- api
