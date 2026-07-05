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
set_prompt_output="$(HOME="$check_home" target/release/windie set "$conversation_id" systemprompt --text "You are concise.")"
if [ "$set_prompt_output" != "set systemprompt $conversation_id" ]; then
    echo "expected set systemprompt to confirm conversation id" >&2
    exit 1
fi
HOME="$check_home" target/release/windie insert "$conversation_id" toolschema --name run_shell --description "Run a shell command" --parameters '{"type":"object"}' >/dev/null
message_id="$(HOME="$check_home" target/release/windie insert "$conversation_id" message --role user --text hello)"
list_json_output="$(HOME="$check_home" target/release/windie ls --json)"
if ! printf '%s\n' "$list_json_output" | grep -q "\"id\": \"$conversation_id\""; then
    echo "expected ls --json to include conversation id" >&2
    exit 1
fi
inspect_json_output="$(HOME="$check_home" target/release/windie inspect "$conversation_id" --json)"
if ! printf '%s\n' "$inspect_json_output" | grep -q "\"conversation_id\": \"$conversation_id\""; then
    echo "expected inspect --json to include conversation id" >&2
    exit 1
fi
if ! printf '%s\n' "$inspect_json_output" | grep -q '"tool_schemas": \['; then
    echo "expected inspect --json to include tool schemas" >&2
    exit 1
fi
if ! printf '%s\n' "$inspect_json_output" | grep -q '"active_path": \['; then
    echo "expected inspect --json to include active path" >&2
    exit 1
fi
if ! printf '%s\n' "$inspect_json_output" | grep -q '"model_context": \['; then
    echo "expected inspect --json to include model context" >&2
    exit 1
fi
bench_conversation_output="$(HOME="$check_home" target/release/windie bench "$conversation_id")"
if ! printf '%s\n' "$bench_conversation_output" | grep -q "mode: conversation"; then
    echo "expected conversation benchmark to print conversation mode" >&2
    exit 1
fi
if ! printf '%s\n' "$bench_conversation_output" | grep -q "active path messages: 1"; then
    echo "expected conversation benchmark to load one active path message" >&2
    exit 1
fi
if ! printf '%s\n' "$bench_conversation_output" | grep -q "tree messages: 1"; then
    echo "expected conversation benchmark to load one tree message" >&2
    exit 1
fi
if ! printf '%s\n' "$bench_conversation_output" | grep -q "tool schema load:"; then
    echo "expected conversation benchmark to include tool schema load" >&2
    exit 1
fi
bench_report="$check_home/baseline.json"
current_report="$check_home/current.json"
HOME="$check_home" target/release/windie bench "$conversation_id" --runs 2 --json > "$bench_report"
HOME="$check_home" target/release/windie bench "$conversation_id" --runs 2 --json > "$current_report"
if ! grep -q '"runs": 2' "$bench_report"; then
    echo "expected JSON benchmark report to include run count" >&2
    exit 1
fi
if ! grep -q '"format_version": 1' "$bench_report"; then
    echo "expected JSON benchmark report to include format version" >&2
    exit 1
fi
if ! grep -q '"samples": \[' "$bench_report"; then
    echo "expected JSON benchmark report to include samples" >&2
    exit 1
fi
if ! grep -q '"summary": {' "$bench_report"; then
    echo "expected JSON benchmark report to include summary" >&2
    exit 1
fi
if ! grep -q '"active_path_load": {' "$bench_report"; then
    echo "expected JSON benchmark report to include active path load summary" >&2
    exit 1
fi
if ! grep -q '"context_build": {' "$bench_report"; then
    echo "expected JSON benchmark report to include context build summary" >&2
    exit 1
fi
if ! grep -q '"tool_schema_load": {' "$bench_report"; then
    echo "expected JSON benchmark report to include tool schema load summary" >&2
    exit 1
fi
bench_compare_output="$(target/release/windie bench compare "$bench_report" "$current_report")"
if ! printf '%s\n' "$bench_compare_output" | grep -q "performance comparison"; then
    echo "expected benchmark compare to print comparison output" >&2
    exit 1
fi
show_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if ! printf '%s\n' "$show_output" | grep -q "user  $message_id  hello"; then
    echo "expected show to include inserted message" >&2
    exit 1
fi
assistant_message_id="$(HOME="$check_home" target/release/windie insert "$conversation_id" message --role assistant --text hello-back)"
third_message_id="$(HOME="$check_home" target/release/windie insert "$conversation_id" message --role user --text next)"
tree_output="$(HOME="$check_home" target/release/windie tree "$conversation_id")"
if ! printf '%s\n' "$tree_output" | grep -q "\\* user  $third_message_id  next"; then
    echo "expected tree to mark latest inserted message as active" >&2
    exit 1
fi
HOME="$check_home" target/release/windie activate "$conversation_id" "$assistant_message_id" >/dev/null
active_show_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if ! printf '%s\n' "$active_show_output" | grep -q "assistant  $assistant_message_id  hello-back"; then
    echo "expected show to include activated path" >&2
    exit 1
fi
if printf '%s\n' "$active_show_output" | grep -q "$third_message_id"; then
    echo "expected show to exclude inactive branch" >&2
    exit 1
fi
forked_conversation_id="$(HOME="$check_home" target/release/windie fork "$conversation_id" "$assistant_message_id")"
forked_show_output="$(HOME="$check_home" target/release/windie show "$forked_conversation_id")"
if ! printf '%s\n' "$forked_show_output" | grep -q "assistant  .*  hello-back"; then
    echo "expected fork to include messages through fork point" >&2
    exit 1
fi
if printf '%s\n' "$forked_show_output" | grep -q "next"; then
    echo "expected fork to exclude messages after fork point" >&2
    exit 1
fi
HOME="$check_home" target/release/windie truncate "$conversation_id" "$assistant_message_id" >/dev/null
truncated_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if printf '%s\n' "$truncated_output" | grep -q "$third_message_id"; then
    echo "expected truncate to prune descendants after checkpoint" >&2
    exit 1
fi
HOME="$check_home" target/release/windie update "$conversation_id" message "$message_id" --text hi >/dev/null
HOME="$check_home" target/release/windie set "$conversation_id" systemprompt --text "You are direct." >/dev/null
updated_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if ! printf '%s\n' "$updated_output" | grep -q "user  $message_id  hi"; then
    echo "expected show to include updated message" >&2
    exit 1
fi
HOME="$check_home" target/release/windie rm "$conversation_id" message "$assistant_message_id" >/dev/null
HOME="$check_home" target/release/windie rm "$conversation_id" message "$message_id" >/dev/null
removed_message_output="$(HOME="$check_home" target/release/windie show "$conversation_id")"
if [ "$removed_message_output" != "no messages" ]; then
    echo "expected rm message to remove the only message" >&2
    exit 1
fi
HOME="$check_home" target/release/windie rm "$conversation_id" >/dev/null
removed_conversation_output="$(HOME="$check_home" target/release/windie ls)"
if printf '%s\n' "$removed_conversation_output" | grep -q "$conversation_id"; then
    echo "expected rm conversation to remove original conversation" >&2
    exit 1
fi
if ! printf '%s\n' "$removed_conversation_output" | grep -q "$forked_conversation_id"; then
    echo "expected rm conversation to leave forked conversation" >&2
    exit 1
fi
HOME="$check_home" target/release/windie rm "$forked_conversation_id" >/dev/null
removed_fork_output="$(HOME="$check_home" target/release/windie ls)"
if [ "$removed_fork_output" != "no conversations" ]; then
    echo "expected rm conversation to remove forked conversation" >&2
    exit 1
fi
fixture_conversation_id="$(HOME="$check_home" WINDIE_BIN="$PWD/target/release/windie" scripts/create_benchmark_fixture.sh stress-100)"
fixture_bench_output="$(HOME="$check_home" target/release/windie bench "$fixture_conversation_id")"
if ! printf '%s\n' "$fixture_bench_output" | grep -q "active path messages: 100"; then
    echo "expected stress fixture active path to have 100 messages" >&2
    exit 1
fi
if ! printf '%s\n' "$fixture_bench_output" | grep -q "tree messages: 164"; then
    echo "expected stress fixture tree to have 164 messages" >&2
    exit 1
fi
HOME="$check_home" target/release/windie rm "$fixture_conversation_id" >/dev/null
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
target/release/windie bench >/dev/null
bare_bench_exit_code=$?
set -e

if [ "$bare_bench_exit_code" -ne 2 ]; then
    echo "expected bench without conversation id to exit 2, got $bare_bench_exit_code" >&2
    exit 1
fi
set +e
target/release/windie bench ls >/dev/null
bench_list_exit_code=$?
set -e

if [ "$bench_list_exit_code" -ne 2 ]; then
    echo "expected removed bench ls command to exit 2, got $bench_list_exit_code" >&2
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
target/release/windie new extra >/dev/null
exit_code=$?
set -e

if [ "$exit_code" -ne 2 ]; then
    echo "expected invalid command to exit 2, got $exit_code" >&2
    exit 1
fi
