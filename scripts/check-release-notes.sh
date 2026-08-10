#!/usr/bin/env bash

set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
changelog="$project_root/CHANGELOG.md"
release_notes="$project_root/RELEASE_NOTES.md"

headings() {
  sed -nE 's/^## \[([^]]+)\].*$/\1/p' "$1"
}

extract_section() {
  local file="$1"
  local version="$2"

  awk -v version="$version" '
    $0 ~ "^## \\[" version "\\]" { found = 1; next }
    found && /^## \[/ { exit }
    found { print }
  ' "$file"
}

if ! diff -u <(headings "$changelog") <(headings "$release_notes"); then
  echo "CHANGELOG.md and RELEASE_NOTES.md must contain the same release sections." >&2
  exit 1
fi

while IFS= read -r version; do
  [[ -n "$version" ]] || continue

  for file in "$changelog" "$release_notes"; do
    section="$(extract_section "$file" "$version")"
    if [[ -z "${section//[[:space:]]/}" ]]; then
      echo "$file has an empty [$version] section." >&2
      exit 1
    fi
  done
done < <(headings "$changelog")

echo "Release sections are present and non-empty in both changelog files."
