# CLI Commands

The Windie CLI is an explicit client of the same operations used by the API.
It parses one command, performs it, prints terminal or JSON output, and exits.

## Process-Level Commands

| Command | Behavior |
| --- | --- |
| `windie` | Exit successfully without taking runtime action |
| `windie -h` / `windie --help` | Print accepted syntax and notes |
| `windie -V` / `windie --version` | Print package version |
| `windie api` | Start the authenticated localhost API and bundled operator UI |
| `windie doctor` | Inspect paths and external integration prerequisites without starting them |
| `windie status` | Report whether the local Bifrost health endpoint is available |

`windie api` listens on the configured localhost address, creates a per-process
API token, prints the authenticated UI URL, owns the persistent MCP registry,
and recovers only runtime records whose ownership leases have expired.

## Gateway and Models

### `windie gateway start`

Starts Bifrost only when it is not already healthy. Windie uses an explicit
development binary when configured, otherwise the pinned public npm or Docker
launcher.

### `windie gateway stop`

Stops the identifiable local Bifrost process/container and waits for the health
endpoint to disappear.

### `windie models`

Requires Bifrost to be running, loads the gateway model catalog, sorts model
IDs, and prints them. It does not start or reconfigure Bifrost.

## Conversation Creation and Listing

### `windie new`

Creates an empty conversation using Windie's hardcoded default model and prints
the generated ID.

### `windie ls`

Lists lightweight conversation summaries without loading message bodies.

### `windie ls --json`

Prints the same summaries as stable machine-readable JSON.

## Conversation Inspection

### `windie show <conversation_id>`

Prints only the selected active path.

### `windie tree <conversation_id>`

Prints the complete stored message tree and marks the active message.

### `windie inspect <conversation_id> --json`

Prints a full read-only snapshot containing the tree, active path,
conversation-level settings, attached schemas, exact model context, and latest
compaction.

### `windie inspect <conversation_id> --json --model <provider/model>`

Uses the supplied model only in the inspection report. It does not persist that
model and does not query it.

## Setting the Active Path

### `windie activate <conversation_id> <message_id>`

Stores the selected message as active. Windie derives the path through parent
links. No messages are copied, inserted, or deleted.

## Inserting Messages

Syntax:

```text
windie insert <conversation_id> message \
  --role <user|assistant|tool> \
  [--text <text>] [--image <path>]...
```

Text and image flags may be repeated and their order becomes message-part
order. At least one non-empty text or image part is required.

The parser recognizes role `tool`, but the shared operation rejects direct tool
messages. Tool results must be created by `approve` or `deny`.

Multipart messages, including any image or several parts, are restricted to
the user role. A simple one-part assistant text message can be inserted.

Insertion appends below the active message and makes the new message active.

## Updating Messages

```text
windie update <conversation_id> message <message_id> --text <new_text>
```

Replaces visible text in place. It preserves role, parent, metadata,
descendants, images, and active selection. It clears compactions.

## Removing Messages and Conversations

### `windie rm <conversation_id>`

Deletes the conversation's messages, compactions, attached schemas, and
orphaned images. Durable runtime runs, events, and tool execution claims are
deleted through their database cascades in the same operation.

### `windie rm <conversation_id> message <message_id>`

Splice-deletes a normal node and promotes its children. Removing a tool-call or
tool-output node deletes the complete call/output group.

### `windie truncate <conversation_id> <message_id>`

Keeps the selected message and recursively removes every descendant.

### `windie fork <conversation_id> <message_id>`

Creates a new conversation containing the source path through the selected
message and prints the new conversation ID. It copies model, reasoning effort,
approval mode, messages, metadata, and image content, but not system prompt,
attached tools, or compactions.

## System Prompt and Model

### `windie set <conversation_id> systemprompt --text <text>`

Sets or replaces the conversation-level system prompt. Passing an empty string
clears it.

### `windie rm <conversation_id> systemprompt`

Explicitly clears the system prompt without changing message nodes.

### `windie set <conversation_id> model <provider/model>`

Persists the default model for future queries. Changing the model clears the
stored reasoning effort.

There is currently no CLI command for setting reasoning effort or tool approval
mode; those mutations are exposed through the API/inspector.

## Provider Tool Catalog and Attachment

### `windie tools`

Lists tools from every available code-approved provider. Providers that fail to
start or list are skipped.

### `windie tools <provider_id>`

The parser supports this more specific form even though the help text only
shows `windie tools`. It lists one provider and surfaces that provider's error.

### `windie attach <conversation_id> tool <provider_id> <tool_name>`

Loads the provider definition, persists its schema and provider mapping on the
conversation, and prints the model-facing schema name.

### `windie detach <conversation_id> tool <schema_name>`

Removes one provider-backed attachment by model-facing schema name.

## Raw Tool Schemas

### Insert

```text
windie insert <conversation_id> toolschema \
  --name <name> \
  --description <text> \
  --parameters <json>
```

All three flags are required exactly once. The name must meet the
OpenAI-compatible function-name rules, the description must be non-empty, and
parameters must decode as a JSON object.

The schema is model-facing but has no executable provider.

### Update

```text
windie update <conversation_id> toolschema <current_name> \
  --name <new_name> \
  --description <text> \
  --parameters <json>
```

Replaces the schema and may rename it. Updating through this raw primitive
produces a manual, non-executable attachment.

### Remove

```text
windie rm <conversation_id> toolschema <name>
```

Deletes the schema from future model requests.

## Tool Approval Commands

### `windie approvals <conversation_id>`

Lists the next active-path call requiring manual approval. At most one pending
approval is exposed because calls must resolve in assistant metadata order.
The output includes both the assistant message ID and tool-call ID required by
the decision commands.

### `windie approve <conversation_id> <assistant_message_id> <tool_call_id>`

Executes the next approved call through a short-lived provider registry and
stores its linked tool output. The two IDs identify the branch independently
from the current selection. This command does not automatically issue the next
model query.

### `windie deny <conversation_id> <assistant_message_id> <tool_call_id>`

Stores a linked failed output saying the user rejected the call. It does not
invoke the provider or automatically query again.

## Querying

### `windie query <conversation_id>`

Requires Bifrost, uses the conversation's persisted model and reasoning effort,
captures the selected path as its execution cursor, and streams assistant
output to the terminal.

Runtime automatically resolves policy-denied and auto-approved attached calls.
It stops when a manual approval is needed or when an assistant response has no
tool calls.

### `windie query <conversation_id> --model <provider/model>`

Uses a one-request model override. The conversation default is unchanged.

The CLI has no query flag for a one-request reasoning override.

## Benchmarks

### Conversation benchmark

```text
windie bench <conversation_id> [--runs <positive_integer>] [--json]
```

Measures store opening, path/tree/tool-schema loading, and context construction
for a real conversation without querying a provider.

### Runtime benchmark

```text
windie bench runtime [--runs <positive_integer>] [--json]
```

Measures generated provider-free runtime, tree mutation, context, and fake MCP
primitives.

### Live benchmark

```text
windie bench live [--runs <positive_integer>] [--json]
```

Requires Bifrost and sends a real model request. It may cost money.

With one run and no JSON flag, benchmarks print a human-readable baseline.
Repeated runs produce summary statistics. `--json` writes a persistent report
to stdout.

### Compare reports

```text
windie bench compare <baseline.json> <current.json>
```

Loads two reports and prints median differences. Compare does not accept
`--runs` or `--json`.

## Invalid Usage

Unsupported argument shapes print usage and exit with status 2. Commands do not
accept free-form reordered flags unless their parser explicitly supports them.

## Relevant Code

- `src/cli.rs`
- `src/main.rs`
- `src/output.rs`
- `src/operation.rs`
