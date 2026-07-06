#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

perf_dir="${WINDIE_PERF_DIR:-.windie/perf}"
baseline_report="$perf_dir/baseline.json"
current_report="$perf_dir/current.json"
runtime_baseline_report="$perf_dir/runtime-baseline.json"
runtime_current_report="$perf_dir/runtime-current.json"
conversation_comparison_report="$perf_dir/conversation-comparison.txt"
runtime_comparison_report="$perf_dir/runtime-comparison.txt"
message_file="$perf_dir/commit-message.txt"

if [ ! -f "$current_report" ] && [ ! -f "$runtime_current_report" ]; then
    echo "no current benchmark reports to promote"
    exit 0
fi

mkdir -p "$perf_dir"
if [ -f "$current_report" ]; then
    mv "$current_report" "$baseline_report"
    echo "promoted current conversation benchmark report to local baseline: $baseline_report"
fi
if [ -f "$runtime_current_report" ]; then
    mv "$runtime_current_report" "$runtime_baseline_report"
    echo "promoted current runtime benchmark report to local baseline: $runtime_baseline_report"
fi

rm -f "$conversation_comparison_report" "$runtime_comparison_report" "$message_file"
