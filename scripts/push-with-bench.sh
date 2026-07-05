#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

git push "$@"
scripts/promote-bench-baseline.sh
