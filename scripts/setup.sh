#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT/dev/windie-inspector"

for command in cargo npm; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command not found: $command" >&2
    exit 1
  fi
done

echo "Installing frontend dependencies with npm..."
cd "$UI"
npm ci --legacy-peer-deps

echo
echo "Windie setup complete."
echo "Start development: scripts/dev.sh"
echo "Check the checkout: scripts/check.sh"
