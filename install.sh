#!/bin/sh
set -eu

repo="${WINDIE_REPO:-buiilding/Windie-Sandbox}"
install_dir="${WINDIE_INSTALL_DIR:-$HOME/.local/bin}"
windie_home="${WINDIE_HOME:-$HOME/.windie}"
api_address="127.0.0.1:8787"
inspector_address="127.0.0.1:3000"
gateway_address="127.0.0.1:8080"

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

for binary in windie bifrost windie-inspector; do
  if [ ! -f "$tmp_dir/$binary" ]; then
    echo "release asset did not contain $binary binary" >&2
    exit 1
  fi
done

install -m 0755 "$tmp_dir/windie" "$install_dir/windie"
install -m 0755 "$tmp_dir/bifrost" "$install_dir/bifrost"
install -m 0755 "$tmp_dir/windie-inspector" "$install_dir/windie-inspector"

gateway_health_url="http://$gateway_address/health"
api_health_url="http://$api_address/api/health"
inspector_health_url="http://$inspector_address/"

if ! curl -fsS "$gateway_health_url" >/dev/null 2>&1; then
  WINDIE_BIFROST_BIN="$install_dir/bifrost" "$install_dir/windie" gateway start
fi

if ! curl -fsS "$api_health_url" >/dev/null 2>&1; then
  "$install_dir/windie" api start
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
  echo "api output: $windie_home/windie-api.log" >&2
  exit 1
fi

if ! curl -fsS "$inspector_health_url" >/dev/null 2>&1; then
  "$install_dir/windie" inspector start
fi

i=0
while [ "$i" -lt 30 ]; do
  if curl -fsS "$inspector_health_url" >/dev/null 2>&1; then
    break
  fi
  i=$((i + 1))
  sleep 1
done

if ! curl -fsS "$inspector_health_url" >/dev/null 2>&1; then
  echo "windie installed, but the Inspector did not start" >&2
  echo "Inspector output: $windie_home/windie-inspector.log" >&2
  exit 1
fi

ui_url="http://$inspector_address"
case "$os" in
  darwin)
    open "$ui_url" >/dev/null 2>&1 || true
    nohup "$install_dir/windie" tray >/dev/null 2>&1 </dev/null &
    ;;
  linux)
    if command -v xdg-open >/dev/null 2>&1; then
      xdg-open "$ui_url" >/dev/null 2>&1 || true
    fi
    ;;
esac

echo "windie installed at $install_dir/windie"
echo "Windie tray available as: $install_dir/windie tray"
echo "bundled Bifrost installed at $install_dir/bifrost"
echo "Inspector installed at $install_dir/windie-inspector"
echo "Windie home ready at $windie_home"
echo "provider keys file: $windie_home/.env"
echo "Bifrost: http://$gateway_address"
echo "Windie API: http://$api_address"
echo "Inspector: $ui_url"
