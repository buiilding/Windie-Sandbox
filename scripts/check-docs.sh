#!/usr/bin/env bash

set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
required_files=(
  "README.md"
  "CONTRIBUTING.md"
  "docs/README.md"
  "docs/guides/development/README.md"
  "docs/guides/development/MACOS.md"
  "docs/guides/development/WINDOWS.md"
  "docs/guides/development/LINUX.md"
)

for relative_path in "${required_files[@]}"; do
  if [[ ! -f "$project_root/$relative_path" ]]; then
    echo "required documentation is missing: $relative_path" >&2
    exit 1
  fi
done

rust_version="$(sed -nE 's/^channel = "([^"]+)"$/\1/p' "$project_root/rust-toolchain.toml")"
go_version="$(head -n 1 "$project_root/.go-version")"
node_version="$(head -n 1 "$project_root/vendor/windie-inspector/frontend/.nvmrc")"

for version in "$rust_version" "$go_version" "$node_version"; do
  if [[ -z "$version" ]]; then
    echo "a development toolchain version could not be read" >&2
    exit 1
  fi
  for guide in "${required_files[@]:4}"; do
    if ! grep -Fq "$version" "$project_root/$guide"; then
      echo "$guide does not mention required toolchain version $version" >&2
      exit 1
    fi
  done
done

if grep -RInE '\]\(#\)' "$project_root/README.md" "$project_root/docs"; then
  echo "documentation contains a placeholder Markdown link" >&2
  exit 1
fi

if grep -RInF '<!--' "$project_root/docs/guides"; then
  echo "guides must not contain unfinished template comments" >&2
  exit 1
fi

while IFS=$'\t' read -r source target; do
  case "$target" in
    http://*|https://*|mailto:*|\#*) continue ;;
  esac
  if [[ ! -e "$(dirname "$source")/$target" ]]; then
    echo "broken local documentation link in ${source#$project_root/}: $target" >&2
    exit 1
  fi
done < <(
  while IFS= read -r file; do
    perl -ne 'while (/\]\(([^ )#]+)(?:#[^ )]+)?\)/g) { print "$ARGV\t$1\n"; }' "$file"
  done < <(find "$project_root/docs" -type f -name '*.md' -print; printf '%s\n' "$project_root/README.md" "$project_root/CONTRIBUTING.md")
)

echo "Documentation links and required developer guides are valid."
