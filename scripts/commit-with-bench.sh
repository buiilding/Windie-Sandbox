#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
    cat >&2 <<'USAGE'
usage: scripts/commit-with-bench.sh -m "subject" -m "description" [git commit options]
       scripts/commit-with-bench.sh -F message-file [git commit options]

Runs a local provider-free benchmark, appends the comparison to the commit
message, then runs git commit.
USAGE
}

perf_dir="${WINDIE_PERF_DIR:-.windie/perf}"
runs="${WINDIE_BENCH_RUNS:-5}"
windie_bin="$PWD/target/release/windie"
baseline_report="$perf_dir/baseline.json"
current_report="$perf_dir/current.json"
comparison_report="$perf_dir/comparison.txt"
message_file="$perf_dir/commit-message.txt"

mkdir -p "$perf_dir"

commit_args=()
messages=()
input_message_file=""

while [ "$#" -gt 0 ]; do
    arg="$1"
    case "$arg" in
        -m|--message)
            shift
            if [ "$#" -eq 0 ]; then
                echo "missing message after $arg" >&2
                usage
                exit 2
            fi
            messages+=("$1")
            ;;
        --message=*)
            messages+=("${arg#--message=}")
            ;;
        -m?*)
            messages+=("${arg#-m}")
            ;;
        -am|-ma)
            commit_args+=("-a")
            shift
            if [ "$#" -eq 0 ]; then
                echo "missing message after $arg" >&2
                usage
                exit 2
            fi
            messages+=("$1")
            ;;
        -F|--file)
            shift
            if [ "$#" -eq 0 ]; then
                echo "missing message file after $arg" >&2
                usage
                exit 2
            fi
            input_message_file="$1"
            ;;
        --file=*)
            input_message_file="${arg#--file=}"
            ;;
        -C|-c|--reuse-message|--reedit-message|--fixup|--squash)
            echo "unsupported commit message option for benchmark appending: $arg" >&2
            exit 2
            ;;
        --)
            commit_args+=("$arg")
            shift
            while [ "$#" -gt 0 ]; do
                commit_args+=("$1")
                shift
            done
            break
            ;;
        *)
            commit_args+=("$arg")
            ;;
    esac
    shift
done

if [ "${#messages[@]}" -gt 0 ] && [ -n "$input_message_file" ]; then
    echo "use either -m/--message or -F/--file, not both" >&2
    exit 2
fi

if [ "${#messages[@]}" -eq 0 ] && [ -z "$input_message_file" ]; then
    usage
    exit 2
fi

has_text() {
    [[ "$1" =~ [^[:space:]] ]]
}

print_description_requirement() {
    cat >&2 <<'MESSAGE'
provide an explicit description of that commit. The description should state
what changed, why it changed, and which behavior or code boundary the commit
affects.
MESSAGE
}

message_file_has_body() {
    awk '
        /^[[:space:]]*$/ { next }
        seen_subject == 0 { seen_subject = 1; next }
        { seen_body = 1 }
        END { exit seen_body ? 0 : 1 }
    ' "$1"
}

if [ "${#messages[@]}" -gt 0 ]; then
    if ! has_text "${messages[0]}"; then
        echo "commit message requires a non-empty subject" >&2
        exit 2
    fi

    has_body=0
    if [ "${#messages[@]}" -gt 1 ]; then
        for message in "${messages[@]:1}"; do
            if has_text "$message"; then
                has_body=1
                break
            fi
        done
    fi

    if [ "$has_body" -eq 0 ]; then
        echo "commit message requires an explicit body description" >&2
        print_description_requirement
        echo 'use: scripts/commit-with-bench.sh -m "subject" -m "description"' >&2
        exit 2
    fi
else
    if [ ! -f "$input_message_file" ]; then
        echo "commit message file does not exist: $input_message_file" >&2
        exit 2
    fi

    if ! message_file_has_body "$input_message_file"; then
        echo "commit message file requires a subject and explicit body description" >&2
        print_description_requirement
        exit 2
    fi
fi

run_provider_free_benchmark() {
    output_path="$1"
    bench_home="$(mktemp -d)"
    status=0

    if conversation_id="$(HOME="$bench_home" WINDIE_BIN="$windie_bin" scripts/create_benchmark_fixture.sh stress-100)"; then
        HOME="$bench_home" "$windie_bin" bench "$conversation_id" --runs "$runs" --json > "$output_path" || status=$?
    else
        status=$?
    fi

    rm -rf "$bench_home"
    return "$status"
}

echo "building release binary for benchmark..." >&2
cargo build --release

if [ ! -f "$baseline_report" ]; then
    echo "no local benchmark baseline found; creating $baseline_report" >&2
    run_provider_free_benchmark "$baseline_report"
fi

tmp_current="$current_report.tmp"
echo "running provider-free benchmark ($runs runs)..." >&2
run_provider_free_benchmark "$tmp_current"
mv "$tmp_current" "$current_report"

"$windie_bin" bench compare "$baseline_report" "$current_report" > "$comparison_report"

{
    if [ "${#messages[@]}" -gt 0 ]; then
        first=1
        for message in "${messages[@]}"; do
            if [ "$first" -eq 0 ]; then
                printf '\n'
            fi
            printf '%s\n' "$message"
            first=0
        done
    else
        cat "$input_message_file"
    fi

    printf '\nPerf:\n'
    sed 's/^/  /' "$comparison_report"
} > "$message_file"

echo "benchmark comparison:" >&2
sed 's/^/  /' "$comparison_report" >&2

if [ "${WINDIE_COMMIT_WITH_BENCH_DRY_RUN:-0}" = "1" ]; then
    echo "dry run: commit message written to $message_file" >&2
    exit 0
fi

if [ "${#commit_args[@]}" -gt 0 ]; then
    WINDIE_COMMIT_WITH_BENCH=1 git commit "${commit_args[@]}" -F "$message_file"
else
    WINDIE_COMMIT_WITH_BENCH=1 git commit -F "$message_file"
fi
