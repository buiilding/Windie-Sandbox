#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

WINDIE_PUSH_WITH_BENCH=1 git push "$@"
scripts/promote-bench-baseline.sh
