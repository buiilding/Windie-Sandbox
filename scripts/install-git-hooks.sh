#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

git config core.hooksPath scripts/git-hooks

echo "installed Windie git hooks from scripts/git-hooks"
