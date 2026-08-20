#!/usr/bin/env bash
#
# Builds one Windie release tarball for a single OS/arch target.
#
# Usage: scripts/package-release.sh <rust-target> <asset-label> <dist-dir>
#   <rust-target>  Rust target triple, e.g. x86_64-unknown-linux-gnu
#   <asset-label>  Tarball label, e.g. linux-x86_64
#   <dist-dir>     Output directory for the finished tarball
#
# The script runs on a runner whose OS already matches <rust-target>, so the
# Rust runtime build, the Inspector host build, the embedded UI build, and the
# CGO-backed Bifrost build are all native compilations. Cross-compiling Bifrost
# is intentionally out of scope: it links SQLite through CGO, which needs a
# native C toolchain.
#
# Tarball layout consumed by install.sh:
#   windie            CLI + API server
#   Windie Tray.app   macOS-native notification host (macOS releases only)
#   bifrost           owned Bifrost gateway binary (sibling of `windie`)
#   windie-inspector  standalone Inspector server
#   release-manifest.txt  target/version metadata for installer diagnostics

set -euo pipefail

RUST_TARGET="${1:?usage: package-release.sh <rust-target> <asset-label> <dist-dir>}"
ASSET_LABEL="${2:?usage: package-release.sh <rust-target> <asset-label> <dist-dir>}"
DIST_DIR="${3:?usage: package-release.sh <rust-target> <asset-label> <dist-dir>}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSPECTOR_REPO_DIR="$REPO_ROOT/vendor/windie-inspector"
INSPECTOR_DIR="$INSPECTOR_REPO_DIR/frontend"
INSPECTOR_HOST_MANIFEST="$INSPECTOR_REPO_DIR/host/Cargo.toml"
BIFROST_DIR="$REPO_ROOT/vendor/bifrost"
BIFROST_HTTP_DIR="$BIFROST_DIR/transports/bifrost-http"
BIFROST_BIN="$BIFROST_DIR/tmp/bifrost-http"
BIFROST_VERSION="stable"
VERSION="${GITHUB_REF_NAME:-dev}"

case "$RUST_TARGET" in
  *-apple-darwin) RELEASE_OS="macos" ;;
  *-linux-*) RELEASE_OS="linux" ;;
  *-windows-*) RELEASE_OS="windows" ;;
  *) RELEASE_OS="unknown" ;;
esac
case "$RUST_TARGET" in
  x86_64-*) RELEASE_CPU="x86_64" ;;
  aarch64-*) RELEASE_CPU="aarch64" ;;
  *) RELEASE_CPU="unknown" ;;
esac

STAGING_DIR="$(mktemp -d)"
trap 'rm -rf "$STAGING_DIR"' EXIT

echo "==> windie release: target=$RUST_TARGET label=$ASSET_LABEL version=$VERSION"

WINDIE_BIN="$REPO_ROOT/target/$RUST_TARGET/release/windie"
INSPECTOR_BIN="$REPO_ROOT/target/$RUST_TARGET/release/windie-inspector"

# --- 1. Build or reuse the Inspector host ------------------------------------
# The host is a separate Cargo package. It embeds the UI build, but it is not a
# binary target of the Windie runtime package. Both packages use the repository
# target directory so the release staging path stays predictable.
if [ "${WINDIE_REUSE_INSPECTOR:-0}" = "1" ] && [ -f "$INSPECTOR_BIN" ]; then
  echo "==> reusing cached windie inspector"
else
  # rust-embed captures vendor/windie-inspector/frontend/build at compile time, so the UI must be
  # built before the independent Inspector host package.
  echo "==> building inspector UI"
  npm ci --prefix "$INSPECTOR_DIR" --legacy-peer-deps
  npm run build --prefix "$INSPECTOR_DIR"
  echo "==> building windie inspector host ($RUST_TARGET)"
  cargo build --release --target "$RUST_TARGET" --target-dir "$REPO_ROOT/target" --manifest-path "$INSPECTOR_HOST_MANIFEST"
fi
[ -f "$INSPECTOR_BIN" ] || { echo "windie inspector binary not found at $INSPECTOR_BIN" >&2; exit 1; }

# --- 2. Build the windie binary ----------------------------------------------
echo "==> building windie ($RUST_TARGET)"
cargo build --release --target "$RUST_TARGET" --manifest-path "$REPO_ROOT/Cargo.toml" --bin windie
[ -f "$WINDIE_BIN" ] || { echo "windie binary not found at $WINDIE_BIN" >&2; exit 1; }

# --- 3. Build or reuse the Bifrost binary -----------------------------------
# Bifrost is cached independently per target. Its version is intentionally
# stable because Windie releases do not necessarily change Bifrost.
if [ "${WINDIE_REUSE_BIFROST:-0}" = "1" ] && [ -f "$BIFROST_BIN" ]; then
  echo "==> reusing cached bifrost ($BIFROST_VERSION)"
else
  # main.go has //go:embed all:ui, so the UI must be produced first. `npm run
  # build` (vite) also runs `copy-build`, which places the output in
  # transports/bifrost-http/ui.
  echo "==> building bifrost UI"
  npm ci --prefix "$BIFROST_DIR/ui"
  npm run build --prefix "$BIFROST_DIR/ui"
  [ -f "$BIFROST_HTTP_DIR/ui/index.html" ] || {
    echo "bifrost UI build did not produce $BIFROST_HTTP_DIR/ui/index.html" >&2
    exit 1
  }

  # Build the fork's actual source, not the published modules. The transports
  # go.mod pins released `github.com/maximhq/bifrost/*` versions, so a workspace
  # (go.work) is required to make the local checkouts win. This mirrors
  # `make setup-workspace` followed by the LOCAL=1 native build.
  echo "==> setting up bifrost go workspace (use local modules)"
  (
    cd "$BIFROST_DIR"
    rm -f go.work go.work.sum
    go work init ./cli ./core ./framework ./transports
    for plugin_dir in ./plugins/*/; do
      [ -f "$plugin_dir/go.mod" ] && go work use "$plugin_dir"
    done
    go work sync
  )

  if [ "$RELEASE_OS" = "macos" ]; then
    echo "==> building bifrost (native, CGO static sqlite, macOS 11 deployment target)"
    export MACOSX_DEPLOYMENT_TARGET=11.0
  else
    echo "==> building bifrost (native, CGO static sqlite)"
  fi
  mkdir -p "$BIFROST_DIR/tmp"
  (
    cd "$BIFROST_HTTP_DIR"
    # On macOS, the deployment target above keeps Bifrost compatible with the
    # same baseline as Windie. Without it, the runner SDK can become the
    # binary's minimum supported macOS version.
    CGO_ENABLED=1 go build \
      -ldflags="-w -s -X main.Version=$BIFROST_VERSION" \
      -trimpath \
      -tags "sqlite_static" \
      -o "$BIFROST_BIN" \
      .
  )
fi
[ -f "$BIFROST_BIN" ] || { echo "bifrost binary not found at $BIFROST_BIN" >&2; exit 1; }

# --- 4. Assemble the tarball --------------------------------------------------
echo "==> assembling tarball"
install -m 0755 "$WINDIE_BIN" "$STAGING_DIR/windie"
install -m 0755 "$BIFROST_BIN" "$STAGING_DIR/bifrost"
install -m 0755 "$INSPECTOR_BIN" "$STAGING_DIR/windie-inspector"
TRAY_BUNDLE_NAME="Windie Tray.app"
RELEASE_CONTENTS="windie,bifrost,windie-inspector"
if [ "$RELEASE_OS" = "macos" ]; then
  tray_bundle="$STAGING_DIR/$TRAY_BUNDLE_NAME"
  mkdir -p "$tray_bundle/Contents/MacOS"
  install -m 0755 "$WINDIE_BIN" "$tray_bundle/Contents/MacOS/windie"
  cat > "$tray_bundle/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>windie</string>
  <key>CFBundleIdentifier</key>
  <string>com.windieos.tray</string>
  <key>CFBundleName</key>
  <string>Windie Tray</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
</dict>
</plist>
EOF
  codesign --force --sign - "$tray_bundle"
  RELEASE_CONTENTS="$RELEASE_CONTENTS,$TRAY_BUNDLE_NAME"
fi
cat > "$STAGING_DIR/release-manifest.txt" <<EOF
windie_version=$VERSION
bifrost_version=$BIFROST_VERSION
asset_label=$ASSET_LABEL
rust_target=$RUST_TARGET
os=$RELEASE_OS
cpu=$RELEASE_CPU
inspector_commit=$(git -C "$INSPECTOR_REPO_DIR" rev-parse HEAD)
contents=$RELEASE_CONTENTS
EOF

"$WINDIE_BIN" --version >/dev/null
"$BIFROST_BIN" --help >/dev/null 2>&1 || true

mkdir -p "$DIST_DIR"
TARBALL="$DIST_DIR/windie-$ASSET_LABEL.tar.gz"
if [ "$RELEASE_OS" = "macos" ]; then
  tar -czf "$TARBALL" -C "$STAGING_DIR" windie bifrost windie-inspector "$TRAY_BUNDLE_NAME" release-manifest.txt
else
  tar -czf "$TARBALL" -C "$STAGING_DIR" windie bifrost windie-inspector release-manifest.txt
fi
sha256sum "$TARBALL" > "$TARBALL.sha256"

echo "==> wrote $TARBALL"
echo "==> wrote $TARBALL.sha256"
ls -lh "$TARBALL"
