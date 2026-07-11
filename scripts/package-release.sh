#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-}"
LABEL="${2:-}"
OUTPUT="${3:-$ROOT/dist}"

if [ -z "$TARGET" ] || [ -z "$LABEL" ]; then
  echo "usage: scripts/package-release.sh <rust-target> <artifact-label> [output-dir]" >&2
  exit 2
fi

UI="$ROOT/dev/windie-inspector"
cd "$UI"
npm ci
npm run build

cargo build --release --target "$TARGET" --manifest-path "$ROOT/Cargo.toml"

STAGING="$ROOT/target/package-$LABEL"
ARCHIVE="$OUTPUT/windie-$LABEL.tar.gz"
rm -rf "$STAGING"
mkdir -p "$STAGING/ui" "$OUTPUT"
install -m 755 "$ROOT/target/$TARGET/release/windie" "$STAGING/windie"
cp -R "$UI/build/." "$STAGING/ui/"
tar -C "$STAGING" -czf "$ARCHIVE" .

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$ARCHIVE" > "$ARCHIVE.sha256"
else
  shasum -a 256 "$ARCHIVE" > "$ARCHIVE.sha256"
fi

echo "$ARCHIVE"
