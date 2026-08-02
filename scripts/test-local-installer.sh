#!/usr/bin/env bash
#
# Builds a native local release and installs it through the local copy of the
# public Unix installer. The test uses isolated paths so it does not replace a
# normal Windie installation.
#
# Usage: scripts/test-local-installer.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
landing_root="${WINDIE_LANDING_DIR:-$repo_root/../windie-landing-2nd}"
installer="$landing_root/frontend/public/install"

if [ ! -f "$installer" ]; then
  echo "local Windie installer was not found at $installer" >&2
  exit 1
fi

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) rust_target="x86_64";;
  arm64|aarch64) rust_target="aarch64";;
  *) echo "unsupported architecture: $arch" >&2; exit 1;;
esac

case "$os" in
  darwin)
    rust_target="${rust_target}-apple-darwin"
    asset_label="macos-${arch/arm64/aarch64}"
    ;;
  linux)
    rust_target="${rust_target}-unknown-linux-gnu"
    asset_label="linux-${arch/arm64/aarch64}"
    ;;
  *) echo "unsupported operating system: $os" >&2; exit 1;;
esac

test_root="${WINDIE_LOCAL_TEST_ROOT:-$repo_root/target/local-installer/$asset_label}"
dist_dir="$test_root/dist"
install_dir="$test_root/bin"
windie_home="$test_root/.windie"
asset="$dist_dir/windie-$asset_label.tar.gz"

mkdir -p "$test_root"

if [ -x "$install_dir/windie" ]; then
  echo "==> stopping previous local Windie installation"
  WINDIE_HOME="$windie_home" WINDIE_INSTALL_DIR="$install_dir" \
    "$install_dir/windie" uninstall --yes >/dev/null 2>&1 || true
fi

echo "==> packaging local release"
GITHUB_REF_NAME=local-dev \
WINDIE_REUSE_BIFROST=1 \
WINDIE_REUSE_INSPECTOR=1 \
  "$repo_root/scripts/package-release.sh" "$rust_target" "$asset_label" "$dist_dir"

if [ ! -f "$asset" ] || [ ! -f "$asset.sha256" ]; then
  echo "local release package is incomplete: $asset" >&2
  exit 1
fi

export WINDIE_ASSET_URL="file://$asset"
export WINDIE_CHECKSUM_URL="file://$asset.sha256"
export WINDIE_INSTALL_DIR="$install_dir"
export WINDIE_HOME="$windie_home"

echo "==> running local installer"
curl -fsSL "file://$installer" | sh

echo "==> local installer test completed"
echo "install directory: $install_dir"
echo "Windie home: $windie_home"
