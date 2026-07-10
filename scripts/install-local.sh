#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${WINDIE_INSTALL_ROOT:-$HOME/.local}"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
REVISION="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo local)"
RELEASE_ID="$VERSION-$REVISION"
RELEASES="$PREFIX/lib/windie/releases"
DESTINATION="$RELEASES/$RELEASE_ID"
STAGING="$RELEASES/.$RELEASE_ID-$$"
UI="$ROOT/dev/windie-inspector"

"$ROOT/scripts/check.sh"

cd "$UI"
if [ ! -d node_modules ]; then
  npm install --legacy-peer-deps
fi
npm run build

mkdir -p "$RELEASES" "$PREFIX/bin"
rm -rf "$STAGING"
mkdir -p "$STAGING/ui"
install -m 755 "$ROOT/target/release/windie" "$STAGING/windie"
cp -R "$UI/build/." "$STAGING/ui/"

if [ -e "$DESTINATION" ]; then
  rm -rf "$STAGING"
else
  mv "$STAGING" "$DESTINATION"
fi

ln -s "$DESTINATION/windie" "$PREFIX/bin/.windie-next-$$"
mv -f "$PREFIX/bin/.windie-next-$$" "$PREFIX/bin/windie"

echo "Installed Windie $RELEASE_ID"
echo "Binary: $PREFIX/bin/windie"
echo "The running Windie process is unchanged; restart it to activate this release."
"$PREFIX/bin/windie" doctor

