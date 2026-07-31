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

health_check() {
  curl -fsS --connect-timeout 2 --max-time 3 "$1" >/dev/null 2>&1
}

progress_bar() {
  progress_percent="$1"
  progress_filled=$((progress_percent / 5))
  progress_empty=$((20 - progress_filled))
  progress_filled_bar=""
  progress_empty_bar=""
  progress_index=0
  while [ "$progress_index" -lt "$progress_filled" ]; do
    progress_filled_bar="${progress_filled_bar}#"
    progress_index=$((progress_index + 1))
  done
  progress_index=0
  while [ "$progress_index" -lt "$progress_empty" ]; do
    progress_empty_bar="${progress_empty_bar}-"
    progress_index=$((progress_index + 1))
  done
  printf '\r[%s%s] %3d%%' "$progress_filled_bar" "$progress_empty_bar" "$progress_percent"
}

wait_for_health() {
  health_url="$1"
  max_attempts="$2"
  component_name="$3"
  attempt=0
  progress_bar 80
  while [ "$attempt" -lt "$max_attempts" ]; do
    if health_check "$health_url"; then
      progress_bar 100
      printf '\n'
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 1
  done
  printf '\n'
  echo "windie installed, but $component_name did not start" >&2
  return 1
}

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

echo "Installing LLM gateway"
if ! health_check "$gateway_health_url"; then
  progress_bar 80
  if ! WINDIE_BIFROST_BIN="$install_dir/bifrost" "$install_dir/windie" gateway start >/dev/null 2>&1; then
    printf '\n'
    echo "failed to start the LLM gateway" >&2
    exit 1
  fi
fi
wait_for_health "$gateway_health_url" 30 "the LLM gateway" || exit 1
echo "Started the gateway at http://$gateway_address"

echo "Installing Windie runtime"
if ! health_check "$api_health_url"; then
  progress_bar 80
  if ! "$install_dir/windie" api start >/dev/null 2>&1; then
    printf '\n'
    echo "failed to start the Windie runtime" >&2
    echo "api output: $windie_home/windie-api.log" >&2
    exit 1
  fi
fi
wait_for_health "$api_health_url" 75 "the Windie runtime" || {
  echo "api output: $windie_home/windie-api.log" >&2
  exit 1
}
echo "Started the runtime at http://$api_address"

echo "Installing Windie Inspector UI"
if ! health_check "$inspector_health_url"; then
  progress_bar 80
  if ! "$install_dir/windie" inspector start >/dev/null 2>&1; then
    printf '\n'
    echo "failed to start the Windie Inspector UI" >&2
    echo "Inspector output: $windie_home/windie-inspector.log" >&2
    exit 1
  fi
fi
wait_for_health "$inspector_health_url" 30 "the Windie Inspector UI" || {
  echo "Inspector output: $windie_home/windie-inspector.log" >&2
  exit 1
}
echo "Started the UI at http://$inspector_address"

ui_url="http://$inspector_address"
case "$os" in
  darwin)
    open "$ui_url" >/dev/null 2>&1 || true
    nohup "$install_dir/windie" tray >/dev/null 2>&1 </dev/null &
    echo "Click on the tray on your desktop to manage these processes."
    ;;
  linux)
    if command -v xdg-open >/dev/null 2>&1; then
      xdg-open "$ui_url" >/dev/null 2>&1 || true
    fi
    echo "Manage these processes with: $install_dir/windie gateway|api|inspector start|stop"
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
