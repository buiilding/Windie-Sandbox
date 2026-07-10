#!/usr/bin/env bash
set -euo pipefail

BASE="${WINDIE_BIFROST_URL:-http://127.0.0.1:8080}"
TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT

curl -fsS "$BASE/health" >/dev/null
curl -fsS "$BASE/v1/models" -o "$TEMP/models.json"

MODEL="$(node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
if (!Array.isArray(value.data)) throw new Error("/v1/models did not return a data array");
if (value.data[0]?.id) process.stdout.write(value.data[0].id);
' "$TEMP/models.json")"

if [ -n "$MODEL" ]; then
  LOCAL_MODEL="${MODEL#*/}"
  curl -fsS --get --data-urlencode "model=$LOCAL_MODEL" \
    "$BASE/api/models/parameters" -o "$TEMP/parameters.json"
  node -e '
const fs = require("fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
if (!value || Array.isArray(value) || typeof value !== "object") {
  throw new Error("/api/models/parameters did not return an object");
}
' "$TEMP/parameters.json"
fi

echo "Public Bifrost compatibility check passed${MODEL:+ for $MODEL}."

