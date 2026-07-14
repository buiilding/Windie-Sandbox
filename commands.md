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
prompts, attached tools, gateway lifecycle, and run-owned execution without
shelling out for each operation.

Initial routes:

```text
GET    /api/health
GET    /api/status
GET    /api/models
GET    /api/model-parameters?model=<provider/model>
GET    /api/tools
GET    /api/tools/{provider_id}
POST   /api/gateway/start
POST   /api/gateway/stop
GET    /api/conversations
POST   /api/conversations
GET    /api/conversations/{conversation_id}
DELETE /api/conversations/{conversation_id}
GET    /api/conversations/{conversation_id}/run-approvals
POST   /api/conversations/{conversation_id}/activate
POST   /api/conversations/{conversation_id}/messages
PATCH  /api/conversations/{conversation_id}/messages/{message_id}
DELETE /api/conversations/{conversation_id}/messages/{message_id}
GET    /api/conversations/{conversation_id}/images/{asset_id}
PATCH  /api/conversations/{conversation_id}/system-prompt
DELETE /api/conversations/{conversation_id}/system-prompt
POST   /api/conversations/{conversation_id}/tools
DELETE /api/conversations/{conversation_id}/tools/{schema_name}
POST   /api/conversations/{conversation_id}/tool-schemas
PATCH  /api/conversations/{conversation_id}/tool-schemas/{name}
DELETE /api/conversations/{conversation_id}/tool-schemas/{name}
POST   /api/conversations/{conversation_id}/truncate
POST   /api/conversations/{conversation_id}/fork
POST   /api/conversations/{conversation_id}/input-tokens
POST   /api/conversations/{conversation_id}/runs
GET    /api/runs
GET    /api/runs/{run_id}
GET    /api/runs/{run_id}/approvals
GET    /api/runs/{run_id}/events
POST   /api/runs/{run_id}/stop
POST   /api/runs/{run_id}/approvals/{tool_call_id}/approve
POST   /api/runs/{run_id}/approvals/{tool_call_id}/deny
```

`GET /api/model-parameters` returns normalized read-only metadata for the
selected model, including reasoning effort options and prompt-cache support
when Bifrost reports them.
Windie fetches the source metadata from Bifrost's
`/api/models/parameters?model=<model>` endpoint and does not keep its own
provider/model reasoning or caching table.

Run creation accepts an optional request-scoped reasoning override:

```json
{
  "model": null,
  "reasoning": {
    "effort": "high"
  }
}
```

Omitting `reasoning` keeps the provider/model default for that run.

Provider prompt caching is automatic for model runs when Bifrost metadata
reports `supports_prompt_caching: true`. Windie uses the conversation id as the
stable cache scope. For OpenAI-qualified models, Windie sends
`prompt_cache_key` plus `prompt_cache_retention: "24h"`. For
Anthropic-qualified models, Windie sends `cache_control: {"type":"ephemeral"}`.
If the model is unsupported or metadata lookup fails, Windie sends no cache
fields and the run continues normally.

Run events use server-sent events. `approve` executes and stores the pending
tool result for that run, then continues the run when no later manual approval
is waiting. `deny` stores a rejected tool result and follows the same
continuation rule. Conversations store state; runs own execution and approvals.

## Environment And Installation

```text
windie install <target>
```

Install or verify one approved public runtime dependency. Supported targets:

```text
bifrost
cua-driver
desktop-commander
blender-mcp
brightdata
```

`bifrost`, `desktop-commander`, and `brightdata` use public `npx` packages.
`blender-mcp` uses public `uvx blender-mcp`. `cua-driver` uses the public
trycua installer when `cua-driver` is not already on `PATH`.

```text
windie env OPENAI_API_KEY=<key>
windie env OPENROUTER_API_KEY=<key>
windie env list
windie env unset OPENAI_API_KEY
windie env path
```

Edit Windie's provider-key environment file at `~/.windie/.env`. `list` prints
key names only and never prints secret values.

## Tools

```text
windie tools
windie tools <provider_id>
```

List provider tools available to attach to conversations.

This is a catalog, not a permission grant. Provider availability does not grant
model access and does not authorize execution. A client can show these tools to
the user, then explicitly attach one to a conversation.

Windie currently includes code-approved MCP providers:

```text
cua-driver          MCP provider launched with `cua-driver mcp`
desktop-commander   MCP provider launched with `npx -y @wonderwhy-er/desktop-commander@latest`
blender-mcp         MCP provider launched with `uvx blender-mcp`
brightdata          MCP provider launched with `npx -y @brightdata/mcp`
```

Install or verify approved public provider dependencies with:

```text
windie install cua-driver
windie install desktop-commander
windie install blender-mcp
windie install brightdata
```

Bright Data also requires a user environment token:

```text
windie env BRIGHTDATA_API_TOKEN=<key>
```

```text
windie attach <conversation_id> tool cua-driver click
windie attach <conversation_id> tool desktop-commander read_file
windie attach <conversation_id> tool blender-mcp get_scene_info
windie attach <conversation_id> tool brightdata search_engine
```

Attach one provider tool to a conversation. Attached tools are the schemas sent
to Bifrost during `query`; approval is still required before execution.

MCP provider tools use provider-prefixed model-facing names. For example,
attaching CUA's `click` tool stores and sends the schema as
`cua_driver__click`, while Windie still executes provider tool `click`.
Attaching Blender MCP's `get_scene_info` tool stores and sends the schema as
`blender_mcp__get_scene_info`, while Windie still executes provider tool
`get_scene_info`.
Attaching Bright Data's `search_engine` tool stores and sends the schema as
`brightdata__search_engine`, while Windie still executes provider tool
`search_engine`.

```text
windie detach <conversation_id> tool cua_driver__click
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

`--model` changes only the model value shown in this inspection output. It does
not rewrite the persisted conversation model.

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

Tool output messages are created only by run approval commands
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
errors, so a run prints the provider rejection if the selected model does not
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

The system prompt is not inserted into the message tree. During a run, Windie
prepends it to the run's selected path before sending context to Bifrost.
Setting the system prompt works on an empty conversation tree and also replaces
an existing system prompt.

```text
windie set <conversation_id> model <provider/model>
```

Persist the conversation model used by future runs, `inspect`, and
developer API calls.

`windie run start <conversation_id> --model <provider/model>` remains a run
override. It does not rewrite the persisted conversation model.

```text
windie rm <conversation_id> systemprompt
```

Remove the conversation-level system prompt without changing messages.

## Raw Tool Schemas

```text
windie insert <conversation_id> toolschema --name test_tool --description "Developer test tool" --parameters '{"type":"object","properties":{"value":{"type":"string"}}}'
```

Insert one raw conversation-level tool schema.

A raw tool schema is a developer escape hatch. It is sent to the model during
a run, but it is attached to the `manual` provider and has no executor unless a
real provider-backed tool is attached through `windie attach`.

The schema name must be 1-64 ASCII letters, numbers, `_`, or `-`. The
description must contain non-whitespace text. `--parameters` must be a JSON
object.

```text
windie update <conversation_id> toolschema test_tool --name test_tool --description "Updated developer test tool" --parameters '{"type":"object","properties":{"value":{"type":"string"}}}'
```

Update one existing raw tool schema. The final `--name` value is the stored name
after the update. Updating through this command keeps the row on the manual
provider path.

```text
windie rm <conversation_id> toolschema test_tool
```

Remove one conversation-level tool schema.

## Tree Control

```text
windie activate <conversation_id> <message_id>
```

Select one message as the active message.

The active message defines the default branch through the conversation tree.
`show`, `insert`, run start without `--head`, and context preview use this
selected path. After a run starts, execution follows the run's stored head
instead of the mutable conversation active path.

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

## Runs

```text
windie run start <conversation_id>
```

Start one execution run from the conversation's active path. Requires the local
Bifrost gateway to already be running.

If the model returns a tool call that requires approval, Windie stores the
assistant tool-call metadata and marks the run as `waiting_for_approval`.

The run-owned tool flow is:

```text
windie run start <conversation_id>
windie run approvals <run_id>
windie run approve <run_id> <tool_call_id>
```

Use `windie run deny <run_id> <tool_call_id>` instead of `approve` to store a
rejected tool result.

If policy denies a requested tool, such as an unknown tool name, Windie records a
failed `role: tool` result automatically during the run. It does not show
that call as an approval because there is no user decision to make.

```text
windie run start <conversation_id> --head <message_id>
```

Start one run from an explicit message head instead of the conversation active
path.

```text
windie run approvals <run_id>
```

List pending model-requested tool calls that require explicit approval for one
run.

```text
windie run approve <run_id> <tool_call_id>
```

Execute one pending approved provider tool call, store the result as a
`role: tool` message, and continue the run. Raw/manual schemas do not have
executors and are denied by policy.

```text
windie run deny <run_id> <tool_call_id>
```

Store a rejected `role: tool` result for one pending tool call without executing
it, then continue the run.

```text
windie run start <conversation_id> --model <provider/model>
```

Start one run using a specific model. Requires the local Bifrost gateway to
already be running.

The model name is passed to Bifrost for this run only. It does not
rewrite the persisted conversation model.

Bifrost must have provider config once for each provider used by Windie. The
provider row names the provider, such as `anthropic`. The key row points to the
environment variable, such as `env.ANTHROPIC_API_KEY`. Use the same pattern for
Gemini, Groq, OpenRouter, and other providers.

Examples:

```text
windie run start <conversation_id> --model openai/gpt-4o-mini
windie run start <conversation_id> --model anthropic/claude-3-5-haiku
windie run start <conversation_id> --model ollama/llama3.2
```

## Runtime Status

```text
windie models
```

List models Bifrost reports through its OpenAI-compatible `/v1/models`
endpoint.

Requires the local Bifrost gateway to already be running. This command is
read-only: it does not start, stop, restart, or reconfigure Bifrost.

After changing Windie's `.env`, restart the gateway explicitly before listing
models again:

```text
windie gateway stop
windie gateway start
windie models
```

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
1. public npx package: npx -y @maximhq/bifrost
2. public Docker image: maximhq/bifrost:latest
```

Windie does not use a sibling Bifrost checkout as a runtime dependency. The
workspace Bifrost source is reference material only.

When Windie starts Bifrost, provider keys come from a Windie `.env` file.
Windie only reads:

```text
~/.windie/.env
```

Use `.env.example` as the non-secret template for `~/.windie/.env`. Do not
commit real provider keys.
For `npx`, Windie also passes `PATH` and `HOME` so Node/npm can launch. These
are process-launch variables, not provider keys.

Detached Bifrost process logs are written to one of:

```text
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
windie bench
```

Run the full local/free benchmark suite. It uses temporary fixture databases,
does not start Bifrost, does not call real MCP providers, and does not send a
provider request.

```text
windie bench --persistence
windie bench --conversation
windie bench --runtime
windie bench --tools
windie bench --mutations
windie bench --mcp
```

Category flags filter the benchmark report. They can be combined.

```text
windie bench --runs 100
```

Run repeated local measurements and print min/median/p95/max summaries.

```text
windie bench --runs 100 --json
```

Run the repeated benchmark and write a persistent JSON benchmark artifact to
stdout.

```text
windie update baseline
```

Run the current local benchmark suite and write
`~/.windie/benchmarks/baseline.json`.

```text
windie compare baseline
```

Run the current local benchmark suite, compare it with
`~/.windie/benchmarks/baseline.json`, and print median percentage changes.
Negative percentages mean the current run is faster. Positive percentages mean
the current run is slower.
