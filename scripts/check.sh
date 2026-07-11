#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT/dev/windie-inspector"

if [ ! -d "$UI/node_modules" ]; then
  echo "Frontend dependencies are missing. Run scripts/setup.sh first." >&2
  exit 1
fi

cd "$ROOT"
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings

cd "$UI"
npm run build
