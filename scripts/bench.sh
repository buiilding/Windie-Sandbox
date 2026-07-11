#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PERF_DIR="${WINDIE_PERF_DIR:-$ROOT/.windie/perf}"
RUNS="${WINDIE_BENCH_RUNS:-100}"
WINDIE_BIN="$ROOT/target/release/windie"

usage() {
  cat >&2 <<'USAGE'
usage:
  scripts/bench.sh runtime [runs]
  scripts/bench.sh conversation <conversation_id> [runs]
  scripts/bench.sh compare <baseline.json> <current.json>
  scripts/bench.sh update-baseline
USAGE
}

validate_runs() {
  case "$1" in
    ''|*[!0-9]*|0)
      echo "benchmark runs must be a positive integer: $1" >&2
      exit 2
      ;;
  esac
}

build_release() {
  cargo build --release --manifest-path "$ROOT/Cargo.toml"
}

compare_if_available() {
  baseline="$1"
  current="$2"

  if [ -f "$baseline" ]; then
    "$WINDIE_BIN" bench compare "$baseline" "$current"
  else
    echo "No baseline found at $baseline"
    echo "Review the report, then run scripts/bench.sh update-baseline to keep it."
  fi
}

command="${1:-}"
case "$command" in
  runtime)
    runs="${2:-$RUNS}"
    if [ "$#" -gt 2 ]; then
      usage
      exit 2
    fi
    validate_runs "$runs"
    mkdir -p "$PERF_DIR"
    build_release
    current="$PERF_DIR/runtime-current.json"
    temp_home="$(mktemp -d)"
    trap 'rm -rf "$temp_home"' EXIT
    HOME="$temp_home" "$WINDIE_BIN" bench runtime --runs "$runs" --json > "$current"
    echo "Runtime benchmark report: $current"
    compare_if_available "$PERF_DIR/runtime-baseline.json" "$current"
    ;;
  conversation)
    conversation_id="${2:-}"
    runs="${3:-$RUNS}"
    if [ -z "$conversation_id" ] || [ "$#" -gt 3 ]; then
      usage
      exit 2
    fi
    validate_runs "$runs"
    mkdir -p "$PERF_DIR"
    build_release
    current="$PERF_DIR/current.json"
    "$WINDIE_BIN" bench "$conversation_id" --runs "$runs" --json > "$current"
    echo "Conversation benchmark report: $current"
    compare_if_available "$PERF_DIR/baseline.json" "$current"
    ;;
  compare)
    if [ "$#" -ne 3 ]; then
      usage
      exit 2
    fi
    build_release
    "$WINDIE_BIN" bench compare "$2" "$3"
    ;;
  update-baseline)
    if [ "$#" -ne 1 ]; then
      usage
      exit 2
    fi
    mkdir -p "$PERF_DIR"
    promoted=0
    if [ -f "$PERF_DIR/current.json" ]; then
      mv "$PERF_DIR/current.json" "$PERF_DIR/baseline.json"
      echo "Updated conversation baseline: $PERF_DIR/baseline.json"
      promoted=1
    fi
    if [ -f "$PERF_DIR/runtime-current.json" ]; then
      mv "$PERF_DIR/runtime-current.json" "$PERF_DIR/runtime-baseline.json"
      echo "Updated runtime baseline: $PERF_DIR/runtime-baseline.json"
      promoted=1
    fi
    if [ "$promoted" -eq 0 ]; then
      echo "No current benchmark reports to promote."
    fi
    ;;
  *)
    usage
    exit 2
    ;;
esac
