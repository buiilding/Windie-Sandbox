# Windie Commands

This file is the concrete CLI command reference. Keep primitive operations here
instead of in `AGENTS.md`.

## No-op

```text
windie
```

Exit successfully without doing anything.

Use this to verify the binary exists without starting chat, creating a
conversation tree, opening Bifrost, or mutating persisted state.

## Help

```text
windie -h
```

Show help.

```text
windie --help
```

Show help.

Use this to print the current CLI surface and command notes.

## Version

```text
windie -V
```

Show version.

```text
windie --version
```

Show version.

Use this to print the package version compiled into the binary.

## Developer API

```text
windie api
```

Start the localhost developer API server at `http://127.0.0.1:8787`.

The server prints a per-process API token at startup. Browser clients must send
that token in the `X-Windie-Api-Token` header. The localhost inspector at
`dev/windie-inspector` can store the token by opening it with
`?windie_token=<printed token>`.

The API is a JSON test harness over Windie's existing runtime and store
primitives. It is intended for local tools such as `dev/windie-inspector` to
test conversation trees, active path selection, message mutation, system
prompts, attached tools, gateway lifecycle, and one-shot queries without shelling
out for each operation.

Initial routes:

```text
GET    /api/health
GET    /api/status
GET    /api/tools
GET    /api/tools/{provider_id}
POST   /api/gateway/start
POST   /api/gateway/stop
GET    /api/conversations
POST   /api/conversations
GET    /api/conversations/{conversation_id}
DELETE /api/conversations/{conversation_id}
GET    /api/conversations/{conversation_id}/approvals
POST   /api/conversations/{conversation_id}/approvals/{tool_call_id}/approve
POST   /api/conversations/{conversation_id}/approvals/{tool_call_id}/deny
POST   /api/conversations/{conversation_id}/activate
POST   /api/conversations/{conversation_id}/messages
PATCH  /api/conversations/{conversation_id}/messages/{message_id}
DELETE /api/conversations/{conversation_id}/messages/{message_id}
PATCH  /api/conversations/{conversation_id}/system-prompt
DELETE /api/conversations/{conversation_id}/system-prompt
POST   /api/conversations/{conversation_id}/tools
DELETE /api/conversations/{conversation_id}/tools/{schema_name}
POST   /api/conversations/{conversation_id}/tool-schemas
PATCH  /api/conversations/{conversation_id}/tool-schemas/{name}
DELETE /api/conversations/{conversation_id}/tool-schemas/{name}
POST   /api/conversations/{conversation_id}/truncate
POST   /api/conversations/{conversation_id}/fork
POST   /api/conversations/{conversation_id}/query
```

## Tools

```text
windie tools
windie tools <provider_id>
```

List provider tools available to attach to conversations.

This is a catalog, not a permission grant. Provider availability does not grant
model access and does not authorize execution. A client can show these tools to
the user, then explicitly attach one to a conversation.

```text
windie attach <conversation_id> tool windie run_shell
```

Attach one provider tool to a conversation. Attached tools are the schemas sent
to Bifrost during `query`; approval is still required before execution.

```text
windie detach <conversation_id> tool run_shell
```

Detach one model-facing tool schema from a conversation. Past tool-call and
tool-result messages stay in history; future calls to the detached schema are
recorded as failed tool results with `Tool is not attached: <name>`.

## Conversations

```text
windie new
```

Create a new empty conversation tree.

Output is only the new conversation tree ID, so the command can be used by
scripts.

```text
windie ls
windie ls --json
```

List all persisted conversation trees.

Output includes each conversation tree ID and message count. If no conversation
trees exist, it prints `no conversations`.

`--json` prints a stable machine-readable object with a `conversations` array
for developer tools.

```text
windie show <conversation_id>
```

Show message previews for the active path in one conversation tree.

Output includes each active-path message role, message ID, and one-line text
preview. If the conversation tree has no active messages, it prints
`no messages`.

```text
windie tree <conversation_id>
```

Show the full message tree for one conversation tree.

Output includes all branches. Indentation shows parent/child structure. `*`
marks the active message.

```text
windie inspect <conversation_id> --json
windie inspect <conversation_id> --json --model <provider/model>
```

Print full read-only runtime state as JSON for developer tools and inspection.

The output includes the conversation ID, active message ID, effective model,
conversation-level system prompt, conversation-level tool schemas, full message
tree, active path, exact model-facing context, and latest compaction checkpoint.
Messages include IDs, parent IDs, role, content, ordered parts, and assistant
metadata. Image parts include asset ID, MIME type, and byte count; raw image
bytes are not printed.

`--model` changes only the model value shown in this inspection output. Windie
does not persist conversation or global model selection yet.

## Messages

```text
windie insert <conversation_id> message --role user --text "hello"
windie insert <conversation_id> message --role user --text "what is this?" --image ./image.png
windie insert <conversation_id> message --role user --text "compare these" --image ./a.png --image ./b.png
windie insert <conversation_id> message --role user --text "first" --image ./a.png --text "second" --image ./b.png
```

Insert one message into a conversation tree without querying the model.

The new message is inserted as a child of the active message and becomes the new
active message. If the conversation tree is empty, the new message becomes the
root.

The role must currently be one of:

```text
user
assistant
```

Tool output messages are created only by `windie approve` or `windie deny`
because they must carry the provider tool-call ID they answer.

Examples:

```text
windie insert <conversation_id> message --role user --text "hello"
windie insert <conversation_id> message --role assistant --text "hello back"
```

The command prints the new message ID.

Each `--text` inserts an ordered text part. Each `--image` copies the image
bytes into Windie's SQLite storage and inserts an ordered image part. Repeating
or interleaving `--text` and `--image` stores multiple parts on the same user
message in flag order. The message row keeps a plain text preview by joining
all text parts with newlines. Windie validates local file readability, size,
basic image extension, and image header. Bifrost/provider owns model capability
errors, so `query` prints the provider rejection if the selected model does not
accept image input.

```text
windie update <conversation_id> message <message_id> --text "new text"
```

Replace one message's text without querying the model.

This mutates only the selected message content. It does not remove later
messages, does not run inference, does not change role, and preserves assistant
metadata such as tool calls, reasoning, refusal, audio, and annotations.

```text
windie set <conversation_id> systemprompt --text "system prompt"
```

Set or replace the conversation-level system prompt.

The system prompt is not inserted into the message tree. During `query`, Windie
prepends it to the active path before sending context to Bifrost. Setting the
system prompt works on an empty conversation tree and also replaces an existing
system prompt.

```text
windie rm <conversation_id> systemprompt
```

Remove the conversation-level system prompt without changing messages.

## Raw Tool Schemas

```text
windie insert <conversation_id> toolschema --name run_shell --description "Run a shell command" --parameters '{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}'
```

Insert one raw conversation-level tool schema.

A raw tool schema is a developer escape hatch. It is sent to the model during
`query`, but it is attached to the `manual` provider and has no executor unless
a real provider-backed tool is attached through `windie attach`.

The schema name must be 1-64 ASCII letters, numbers, `_`, or `-`. The
description must contain non-whitespace text. `--parameters` must be a JSON
object.

```text
windie update <conversation_id> toolschema run_shell --name run_shell --description "Run a shell command on the local machine" --parameters '{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}'
```

Update one existing raw tool schema. The final `--name` value is the stored name
after the update. Updating through this command keeps the row on the manual
provider path.

```text
windie rm <conversation_id> toolschema run_shell
```

Remove one conversation-level tool schema.

## Tree Control

```text
windie activate <conversation_id> <message_id>
```

Select one message as the active message.

The active message defines the current runtime path through the conversation
tree. `show`, `insert`, `query`, and context construction use this selected
path.

```text
windie rm <conversation_id>
```

Remove one conversation tree.

This deletes the conversation tree and all messages/compactions owned by that
conversation tree.

```text
windie rm <conversation_id> message <message_id>
```

Remove one message from a conversation tree.

This splices the selected message out of the tree. Direct children are
reparented to the removed message's parent, and deeper descendants keep their
existing parents. If the removed message is a root, its direct children become
root messages.

Tool-call messages are removed as a group. If the selected message is an
assistant message with tool-call metadata, Windie also removes the linear
`role: tool` result chain below that assistant. If the selected message is one
of those tool-output messages, Windie removes the parent assistant tool-call
message and every result in the chain. Surviving descendants are spliced to the
assistant's parent.

Use `truncate` when you want to delete descendants.

```text
windie truncate <conversation_id> <message_id>
```

Remove all descendant messages after one message in a conversation tree.

The checkpoint message is kept. Its children and deeper descendants are deleted.

```text
windie fork <conversation_id> <message_id>
```

Create a new conversation tree copied from the start of an existing
conversation tree through one message.

The forked conversation tree receives new message IDs and can diverge
independently. The command prints the new conversation tree ID.

## Inference

```text
windie query <conversation_id>
```

Run one model response from the active path and insert the assistant message.
Requires the local Bifrost gateway to already be running.

If the model returns a tool call, Windie stores the assistant tool-call metadata
and stops. Tool execution is an explicit separate primitive.

The composable tool flow is:

```text
windie query <conversation_id>
windie approvals <conversation_id>
windie approve <conversation_id> <tool_call_id>
windie query <conversation_id>
```

Use `windie deny <conversation_id> <tool_call_id>` instead of `approve` to store
a rejected tool result.

If policy denies a requested tool, such as an unknown tool name, Windie records a
failed `role: tool` result automatically during the query turn. It does not show
that call as an approval because there is no user decision to make.

```text
windie approvals <conversation_id>
```

List pending active-path model-requested tool calls that require explicit
user approval. Approvals are derived from persisted messages on the active path:
an assistant tool call is pending when no active-path `role: tool` message has a
matching tool-call ID.

```text
windie approve <conversation_id> <tool_call_id>
```

Execute one pending approved tool call and store the result as a `role: tool`
message. For now, only `run_shell` has a real executor.

```text
windie deny <conversation_id> <tool_call_id>
```

Store a rejected `role: tool` result for one pending tool call without executing
it. Run `windie query <conversation_id>` again after approving or denying to let
the model continue from the tool result.

```text
windie query <conversation_id> --model <provider/model>
```

Run one model response from the active path using a specific model. Requires the
local Bifrost gateway to already be running.

The model name is passed to Bifrost for this one request only. Windie does not
persist conversation or global model selection yet.

Bifrost must have provider config once for each provider used by Windie. The
provider row names the provider, such as `anthropic`. The key row points to the
environment variable, such as `env.ANTHROPIC_API_KEY`. Use the same pattern for
Gemini, Groq, OpenRouter, and other providers.

Examples:

```text
windie query <conversation_id> --model openai/gpt-4o-mini
windie query <conversation_id> --model anthropic/claude-3-5-haiku
windie query <conversation_id> --model ollama/llama3.2
```

## Runtime Status

```text
windie status
```

Check local runtime and gateway readiness.

This currently reports whether the local Bifrost gateway is running.

## Gateway

```text
windie gateway start
```

Start the local Bifrost gateway.

If the gateway is already running, the command reports that instead of starting
a duplicate process.

Launcher order:

```text
1. locally built sibling/workspace Bifrost binary
2. public npx package: npx -y @maximhq/bifrost
3. public Docker image: maximhq/bifrost:latest
```

This means another computer can run Windie without cloning Bifrost, as long as
Node/npm or Docker is installed.

When Windie starts Bifrost, provider keys come from a Windie `.env` file.
Lookup order:

```text
~/.windie/.env
./.env in the Windie project directory
```

Use `.env.example` as the non-secret template. Do not commit real provider keys.
For `npx`, Windie also passes `PATH` and `HOME` so Node/npm can launch. These
are process-launch variables, not provider keys.

Detached Bifrost process logs are written to one of:

```text
../bifrost/windie-gateway.log
~/.windie/bifrost/windie-gateway.log
```

Use this file to inspect gateway startup failures.

```text
windie gateway stop
```

Stop the local Bifrost gateway.

If the gateway is not running, the command reports that instead of failing.

## Benchmarks

```text
windie bench <conversation_id>
```

Run local/free performance baseline for one conversation tree. Measures active
path load, full tree load, tool schema load, and model-facing context build.
Does not start Bifrost and does not send a provider request.

```text
windie bench <conversation_id> --runs 100
```

Run the same local/free conversation benchmark repeatedly and print
min/median/p95/max summaries. Use this when checking whether a local code change
actually made the runtime path faster or slower.

```text
windie bench runtime
```

Run provider-free runtime and write-path benchmarks against temporary fixture
databases. Measures small and scaled query preparation, policy-denied query
preparation, pending tool approval scanning, tool result insertion, explicit
denial persistence, splice remove variants, truncate variants, active-path
scale, context scale, system prompt context, compaction context with a remaining
active-path suffix, image-part context, and completed tool-result chains. Does
not touch user conversations, start Bifrost, run shell commands, or send a
provider request.

```text
windie bench <conversation_id> --runs 100 --json
```

Run the repeated benchmark and write a persistent JSON benchmark artifact to
stdout. Redirect this output to a file when saving a baseline:

```text
windie bench <conversation_id> --runs 100 --json > benches/baseline.json
```

```text
windie bench compare <baseline.json> <current.json>
```

Compare two JSON benchmark artifacts and print median percentage changes.
Negative percentages mean the current run is faster. Positive percentages mean
the current run is slower.

```text
windie bench live
```

Run a tiny live provider benchmark. Requires the local Bifrost gateway and sends
a real provider request, so it may cost money.

Use this only when you intentionally want to measure the provider path.

## Commit Benchmark Workflow

```text
scripts/commit-with-bench.sh -m "commit message"
```

Build the release binary, run the local/free conversation stress benchmark and
runtime benchmark, compare current reports with local baselines, append both
comparisons to the commit message, then run `git commit`. If no local baseline
exists, the script creates one first. Set `WINDIE_BENCH_RUNS=<n>` to change the
default repeated run count.

```text
scripts/push-with-bench.sh
```

Run `git push`. After a successful push, promote current benchmark reports to
their local baselines and remove the current reports.
