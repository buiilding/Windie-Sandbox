#!/bin/sh
set -eu

repo="${WINDIE_REPO:-buiilding/Windie-Sandbox}"
install_dir="${WINDIE_INSTALL_DIR:-$HOME/.local/bin}"
windie_home="${WINDIE_HOME:-$HOME/.windie}"
api_address="127.0.0.1:8787"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac

case "$os" in
  darwin) platform="macos" ;;
  linux) platform="linux" ;;
  *) echo "unsupported operating system: $os" >&2; exit 1 ;;
esac

mkdir -p "$install_dir" "$windie_home/bifrost" "$windie_home/benchmarks"
if [ ! -f "$windie_home/.env" ]; then
  : > "$windie_home/.env"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

asset="windie-$platform-$arch.tar.gz"
url="${WINDIE_ASSET_URL:-https://github.com/$repo/releases/latest/download/$asset}"

curl -fsSL "$url" -o "$tmp_dir/$asset"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"

if [ ! -f "$tmp_dir/windie" ]; then
  echo "release asset did not contain windie binary" >&2
  exit 1
fi

if [ ! -f "$tmp_dir/bifrost" ]; then
  echo "release asset did not contain bifrost binary" >&2
  exit 1
fi

install -m 0755 "$tmp_dir/windie" "$install_dir/windie"
install -m 0755 "$tmp_dir/bifrost" "$install_dir/bifrost"

api_health_url="http://$api_address/api/health"
api_log="$windie_home/windie-api.log"

if ! curl -fsS "$api_health_url" >/dev/null 2>&1; then
  nohup env WINDIE_BIFROST_BIN="$install_dir/bifrost" "$install_dir/windie" api < /dev/null > "$api_log" 2>&1 &
fi

i=0
while [ "$i" -lt 75 ]; do
  if curl -fsS "$api_health_url" >/dev/null 2>&1; then
    break
  fi
  i=$((i + 1))
  sleep 1
done

if ! curl -fsS "$api_health_url" >/dev/null 2>&1; then
  echo "windie installed, but the local API did not start" >&2
  echo "api log: $api_log" >&2
  exit 1
fi

api_token_file="$windie_home/api-token"
if [ ! -s "$api_token_file" ]; then
  echo "windie installed, but the API token file was not created" >&2
  echo "api token file: $api_token_file" >&2
  exit 1
fi

api_token="$(tr -d '\r\n' < "$api_token_file")"
ui_url="http://$api_address/?windie_token=$api_token"

case "$os" in
  darwin)
    open "$ui_url" >/dev/null 2>&1 || true
    ;;
  linux)
    if command -v xdg-open >/dev/null 2>&1; then
      xdg-open "$ui_url" >/dev/null 2>&1 || true
    fi
    ;;
esac

echo "windie installed at $install_dir/windie"
echo "bundled Bifrost installed at $install_dir/bifrost"
echo "windie home ready at $windie_home"
echo "provider keys file: $windie_home/.env"
echo "Bifrost runtime: bundled local binary"
echo "windie api: http://$api_address"
echo "windie ui: $ui_url"
