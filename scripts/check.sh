#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."

# Full local/free verification path. This must not call Bifrost or a model
# provider.
cargo test
cargo build --release

target/release/windie --version
target/release/windie --help >/dev/null
empty_output="$(target/release/windie)"
if [ "$empty_output" != "" ]; then
    echo "expected bare windie to print nothing" >&2
    exit 1
fi
check_home="$(mktemp -d)"
trap 'rm -rf "$check_home"' EXIT
list_output="$(HOME="$check_home" target/release/windie ls)"
if [ "$list_output" != "no conversations" ]; then
    echo "expected list without conversations to print no conversations" >&2
    exit 1
fi
conversation_id="$(HOME="$check_home" target/release/windie new)"
message_id="$(HOME="$check_home" target/release/windie append "$conversation_id" --role user --text hello)"
bench_conversation_output="$(HOME="$check_home" target/release/windie bench "$conversation_id")"
if ! printf '%s\n' "$bench_conversation_output" | grep -q "mode: conversation"; then
    echo "expected conversation benchmark to print conversation mode" >&2
    exit 1
fi
if ! printf '%s\n' "$bench_conversation_output" | grep -q "loaded messages: 1"; then
    echo "expected conversation benchmark to load one message" >&2
    exit 1
fi
show_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if ! printf '%s\n' "$show_output" | grep -q "user  $message_id  hello"; then
    echo "expected show to include appended message" >&2
    exit 1
fi
HOME="$check_home" target/release/windie update "$conversation_id" "$message_id" --text hi >/dev/null
updated_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if ! printf '%s\n' "$updated_output" | grep -q "user  $message_id  hi"; then
    echo "expected show to include updated message" >&2
    exit 1
fi
HOME="$check_home" target/release/windie rm "$conversation_id" "$message_id" >/dev/null
removed_message_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if [ "$removed_message_output" != "no messages" ]; then
    echo "expected rm message to remove the only message" >&2
    exit 1
fi
HOME="$check_home" target/release/windie rm "$conversation_id" >/dev/null
removed_conversation_output="$(HOME="$check_home" target/release/windie ls)"
if [ "$removed_conversation_output" != "no conversations" ]; then
    echo "expected rm conversation to remove conversation" >&2
    exit 1
fi
set +e
HOME="$check_home" target/release/windie show missing >/dev/null 2>&1
missing_show_exit_code=$?
set -e

if [ "$missing_show_exit_code" -ne 1 ]; then
    echo "expected show missing conversation to exit 1, got $missing_show_exit_code" >&2
    exit 1
fi
set +e
target/release/windie show >/dev/null
show_without_id_exit_code=$?
set -e

if [ "$show_without_id_exit_code" -ne 2 ]; then
    echo "expected show without id to exit 2, got $show_without_id_exit_code" >&2
    exit 1
fi
set +e
target/release/windie list >/dev/null
list_exit_code=$?
set -e

if [ "$list_exit_code" -ne 2 ]; then
    echo "expected removed list command to exit 2, got $list_exit_code" >&2
    exit 1
fi
set +e
target/release/windie gateway >/dev/null
gateway_exit_code=$?
set -e

if [ "$gateway_exit_code" -ne 2 ]; then
    echo "expected gateway without action to exit 2, got $gateway_exit_code" >&2
    exit 1
fi
set +e
target/release/windie truncate missing missing >/dev/null
truncate_exit_code=$?
set -e

if [ "$truncate_exit_code" -ne 2 ]; then
    echo "expected removed truncate command to exit 2, got $truncate_exit_code" >&2
    exit 1
fi
set +e
target/release/windie new extra >/dev/null
exit_code=$?
set -e

if [ "$exit_code" -ne 2 ]; then
    echo "expected invalid command to exit 2, got $exit_code" >&2
    exit 1
fi
