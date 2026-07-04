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

## Conversations

```text
windie new
```

Create a new empty conversation tree.

Output is only the new conversation tree ID, so the command can be used by
scripts.

```text
windie ls
```

List all persisted conversation trees.

Output includes each conversation tree ID and message count. If no conversation
trees exist, it prints `no conversations`.

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

## Messages

```text
windie insert <conversation_id> --role user --text "hello"
windie insert <conversation_id> --role user --text "what is this?" --image ./image.png
windie insert <conversation_id> --role user --text "compare these" --image ./a.png --image ./b.png
windie insert <conversation_id> --role user --text "first" --image ./a.png --text "second" --image ./b.png
```

Insert one message into a conversation tree without querying the model.

The new message is inserted as a child of the active message and becomes the new
active message. If the conversation tree is empty, the new message becomes the
root.

The role must currently be one of:

```text
system
user
assistant
tool
```

`tool` currently means a tool output message. It is not a tool call schema or a
request to execute a tool.

Examples:

```text
windie insert <conversation_id> --role user --text "hello"
windie insert <conversation_id> --role assistant --text "hello back"
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
windie update <conversation_id> <message_id> --text "new text"
```

Replace one message's text without querying the model.

This mutates only the selected message content. It does not remove later
messages and does not run inference.

```text
windie set systemprompt <conversation_id> --text "system prompt"
```

Set or replace the conversation-level system prompt.

The system prompt is not inserted into the message tree. During `query`, Windie
prepends it to the active path before sending context to Bifrost. Setting the
system prompt works on an empty conversation tree and also replaces an existing
system prompt.

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
windie rm <conversation_id> <message_id>
```

Remove one message from a conversation tree.

This deletes that message and its descendant subtree.

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

If the model returns tool calls, Windie stores those tool calls on the assistant
message and prints a tool-call summary. Windie does not execute tools yet.

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
path load, full tree load, and model-facing context build. Does not start
Bifrost and does not send a provider request.

```text
windie bench <conversation_id> --runs 100
```

Run the same local/free conversation benchmark repeatedly and print
min/median/p95/max summaries. Use this when checking whether a local code change
actually made the runtime path faster or slower.

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
