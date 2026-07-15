#!/bin/sh
set -eu

repo="${WINDIE_REPO:-buiilding/Windie-Sandbox}"
install_dir="${WINDIE_INSTALL_DIR:-$HOME/.local/bin}"
windie_home="${WINDIE_HOME:-$HOME/.windie}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac

case "$os" in
  darwin|linux) ;;
  *) echo "unsupported operating system: $os" >&2; exit 1 ;;
esac

mkdir -p "$install_dir" "$windie_home/bifrost" "$windie_home/benchmarks"
if [ ! -f "$windie_home/.env" ]; then
  : > "$windie_home/.env"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

asset="windie-$os-$arch.tar.gz"
url="https://github.com/$repo/releases/latest/download/$asset"

curl -fsSL "$url" -o "$tmp_dir/$asset"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"

if [ ! -f "$tmp_dir/windie" ]; then
  echo "release asset did not contain windie binary" >&2
  exit 1
fi

install -m 0755 "$tmp_dir/windie" "$install_dir/windie"

if ! command -v npx >/dev/null 2>&1; then
  echo "windie installed, but npx was not found on PATH" >&2
  echo "install Node.js/npm before starting the Bifrost gateway" >&2
  exit 1
fi

npx --version >/dev/null

echo "windie installed at $install_dir/windie"
echo "windie home ready at $windie_home"
echo "provider keys file: $windie_home/.env"
echo "Bifrost runtime: public npx package @maximhq/bifrost"
