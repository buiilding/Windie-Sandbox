#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export WINDIE_DATA_DIR="${WINDIE_DEV_DATA_DIR:-$ROOT/target/windie-dev-data}"
export WINDIE_CONFIG_DIR="${WINDIE_DEV_CONFIG_DIR:-$ROOT/target/windie-dev-config}"

echo "Windie development data: $WINDIE_DATA_DIR"
echo "The installed Windie runtime and user database are not modified."
exec cargo run --manifest-path "$ROOT/Cargo.toml" -- api

