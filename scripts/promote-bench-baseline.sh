#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

perf_dir="${WINDIE_PERF_DIR:-.windie/perf}"
baseline_report="$perf_dir/baseline.json"
current_report="$perf_dir/current.json"
comparison_report="$perf_dir/comparison.txt"
message_file="$perf_dir/commit-message.txt"

if [ ! -f "$current_report" ]; then
    echo "no current benchmark report to promote"
    exit 0
fi

mkdir -p "$perf_dir"
mv "$current_report" "$baseline_report"
rm -f "$comparison_report" "$message_file"

echo "promoted current benchmark report to local baseline: $baseline_report"
