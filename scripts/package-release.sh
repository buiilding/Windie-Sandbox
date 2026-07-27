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
# Rust build, the embedded-inspector build, and the CGO-backed Bifrost build
# are all native compilations. Cross-compiling Bifrost is intentionally out of
# scope: it links SQLite through CGO, which needs a native C toolchain.
#
# Tarball layout consumed by install.sh:
#   windie            CLI + embedded API/inspector
#   bifrost           owned Bifrost gateway binary (sibling of `windie`)

set -euo pipefail

RUST_TARGET="${1:?usage: package-release.sh <rust-target> <asset-label> <dist-dir>}"
ASSET_LABEL="${2:?usage: package-release.sh <rust-target> <asset-label> <dist-dir>}"
DIST_DIR="${3:?usage: package-release.sh <rust-target> <asset-label> <dist-dir>}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSPECTOR_DIR="$REPO_ROOT/dev/windie-inspector"
BIFROST_DIR="$REPO_ROOT/vendor/bifrost"
BIFROST_HTTP_DIR="$BIFROST_DIR/transports/bifrost-http"
VERSION="${GITHUB_REF_NAME:-dev}"

STAGING_DIR="$(mktemp -d)"
trap 'rm -rf "$STAGING_DIR"' EXIT

echo "==> windie release: target=$RUST_TARGET label=$ASSET_LABEL version=$VERSION"

# --- 1. Build the inspector UI (embedded into the windie binary) -------------
# rust-embed captures dev/windie-inspector/build at compile time, so the UI
# must be built before cargo.
echo "==> building inspector UI"
npm ci --prefix "$INSPECTOR_DIR" --legacy-peer-deps
npm run build --prefix "$INSPECTOR_DIR"

# --- 2. Build the windie binary ----------------------------------------------
echo "==> building windie ($RUST_TARGET)"
cargo build --release --target "$RUST_TARGET" --manifest-path "$REPO_ROOT/Cargo.toml"
WINDIE_BIN="$REPO_ROOT/target/$RUST_TARGET/release/windie"
[ -f "$WINDIE_BIN" ] || { echo "windie binary not found at $WINDIE_BIN" >&2; exit 1; }

# --- 3. Build the Bifrost UI, then the Bifrost binary ------------------------
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

echo "==> building bifrost (native, CGO static sqlite, local workspace)"
mkdir -p "$BIFROST_DIR/tmp"
(
  cd "$BIFROST_HTTP_DIR"
  CGO_ENABLED=1 go build \
    -ldflags="-w -s -X main.Version=$VERSION" \
    -trimpath \
    -tags "sqlite_static" \
    -o "$BIFROST_DIR/tmp/bifrost-http" \
    .
)
BIFROST_BIN="$BIFROST_DIR/tmp/bifrost-http"
[ -f "$BIFROST_BIN" ] || { echo "bifrost binary not found at $BIFROST_BIN" >&2; exit 1; }

# --- 4. Assemble the tarball --------------------------------------------------
echo "==> assembling tarball"
install -m 0755 "$WINDIE_BIN" "$STAGING_DIR/windie"
install -m 0755 "$BIFROST_BIN" "$STAGING_DIR/bifrost"

mkdir -p "$DIST_DIR"
TARBALL="$DIST_DIR/windie-$ASSET_LABEL.tar.gz"
tar -czf "$TARBALL" -C "$STAGING_DIR" windie bifrost

echo "==> wrote $TARBALL"
ls -lh "$TARBALL"
