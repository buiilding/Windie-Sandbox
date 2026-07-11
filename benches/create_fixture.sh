#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."

fixture="${1:-}"
if [ "$fixture" != "stress-100" ]; then
    echo "usage: benches/create_fixture.sh stress-100" >&2
    exit 2
fi

WINDIE_BIN="${WINDIE_BIN:-windie}"

run_windie() {
    "$WINDIE_BIN" "$@"
}

conversation_id="$(run_windie new)"
run_windie set "$conversation_id" systemprompt --text "You are a concise local runtime benchmark model." >/dev/null
run_windie insert "$conversation_id" toolschema --name run_shell --description "Run a shell command" --parameters '{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}' >/dev/null

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
image_a="$tmp_dir/a.png"
image_b="$tmp_dir/b.gif"

# Windie validates image headers before storing bytes. These tiny files are
# intentionally minimal because benchmark fixtures measure storage/context work,
# not image decoding.
printf '\211PNG\r\n\032\n' > "$image_a"
printf 'GIF89a' > "$image_b"

insert_text() {
    role="$1"
    text="$2"

    run_windie insert "$conversation_id" message --role "$role" --text "$text"
}

activate_message() {
    message_id="$1"

    run_windie activate "$conversation_id" "$message_id" >/dev/null
}

branch_chain() {
    checkpoint_id="$1"
    label="$2"
    count="$3"

    activate_message "$checkpoint_id"

    index=1
    while [ "$index" -le "$count" ]; do
        case $((index % 3)) in
            0) role="assistant" ;;
            1) role="user" ;;
            *) role="assistant" ;;
        esac

        insert_text "$role" "$label branch message $index" >/dev/null
        index=$((index + 1))
    done
}

i=1
while [ "$i" -le 100 ]; do
    case "$i" in
        4)
            message_id="$(insert_text user "main inserted note $i")"
            ;;
        5)
            message_id="$(run_windie insert "$conversation_id" message --role user --image "$image_a")"
            ;;
        10)
            message_id="$(run_windie insert "$conversation_id" message --role user --text "main text plus image $i" --image "$image_a")"
            ;;
        15)
            message_id="$(run_windie insert "$conversation_id" message --role user --image "$image_a" --image "$image_b")"
            ;;
        20)
            message_id="$(run_windie insert "$conversation_id" message --role user --text "main first text $i" --image "$image_a" --text "main second text $i" --image "$image_b")"
            ;;
        *)
            case $((i % 4)) in
                0) role="assistant" ;;
                1) role="user" ;;
                2) role="assistant" ;;
                *) role="user" ;;
            esac
            message_id="$(insert_text "$role" "main $role message $i")"
            ;;
    esac

    case "$i" in
        10) checkpoint_10="$message_id" ;;
        25) checkpoint_25="$message_id" ;;
        40) checkpoint_40="$message_id" ;;
        55) checkpoint_55="$message_id" ;;
        70) checkpoint_70="$message_id" ;;
        85) checkpoint_85="$message_id" ;;
        100) final_message_id="$message_id" ;;
    esac

    i=$((i + 1))
done

branch_chain "$checkpoint_10" "short" 4
branch_chain "$checkpoint_25" "alpha" 12
branch_chain "$checkpoint_40" "beta" 12
branch_chain "$checkpoint_55" "gamma" 12
branch_chain "$checkpoint_70" "delta" 12
branch_chain "$checkpoint_85" "epsilon" 12

activate_message "$final_message_id"

printf '%s\n' "$conversation_id"
