#!/usr/bin/env bash
set -euo pipefail

REPOSITORY="${WINDIE_GITHUB_REPOSITORY:-buiilding/Windie-Sandbox}"
PREFIX="${WINDIE_INSTALL_ROOT:-$HOME/.local}"
VERSION="${WINDIE_VERSION:-latest}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) LABEL="macos-aarch64" ;;
  Darwin-x86_64) LABEL="macos-x86_64" ;;
  Linux-x86_64) LABEL="linux-x86_64" ;;
  Linux-aarch64|Linux-arm64) LABEL="linux-aarch64" ;;
  *)
    echo "Windie does not publish a binary for $(uname -s) $(uname -m) yet." >&2
    exit 1
    ;;
esac

if [ "$VERSION" = "latest" ]; then
  BASE_URL="https://github.com/$REPOSITORY/releases/latest/download"
else
  BASE_URL="https://github.com/$REPOSITORY/releases/download/$VERSION"
fi

ARCHIVE="windie-$LABEL.tar.gz"
TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT

curl -fL "$BASE_URL/$ARCHIVE" -o "$TEMP/$ARCHIVE"
curl -fL "$BASE_URL/$ARCHIVE.sha256" -o "$TEMP/$ARCHIVE.sha256"

EXPECTED="$(awk '{print $1}' "$TEMP/$ARCHIVE.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$TEMP/$ARCHIVE" | awk '{print $1}')"
else
  ACTUAL="$(shasum -a 256 "$TEMP/$ARCHIVE" | awk '{print $1}')"
fi
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "Windie archive checksum did not match." >&2
  exit 1
fi

mkdir -p "$TEMP/extracted"
tar -C "$TEMP/extracted" -xzf "$TEMP/$ARCHIVE"
RELEASE_ID="$($TEMP/extracted/windie --version | awk '{print $2}')-$LABEL"
DESTINATION="$PREFIX/lib/windie/releases/$RELEASE_ID"
mkdir -p "$PREFIX/lib/windie/releases" "$PREFIX/bin"
rm -rf "$DESTINATION"
mv "$TEMP/extracted" "$DESTINATION"
ln -s "$DESTINATION/windie" "$PREFIX/bin/.windie-next-$$"
mv -f "$PREFIX/bin/.windie-next-$$" "$PREFIX/bin/windie"

echo "Installed Windie at $PREFIX/bin/windie"
case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Add $PREFIX/bin to PATH." ;;
esac
"$PREFIX/bin/windie" doctor
