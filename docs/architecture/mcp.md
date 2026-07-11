# MCP Architecture

Windie's MCP layer turns approved stdio MCP servers into provider-neutral tools
without exposing JSON-RPC, child-process, or session details to runtime.

## Layered Flow

```text
runtime
  |
  | AttachedTool + ToolCall
  v
ToolProviderRegistry
  |
  | select provider by provider ID/kind
  v
McpToolProvider
  |
  | prepare provider, map names, normalize result
  v
McpSessionPool or one-shot MCP call
  |
  | initialize + JSON-RPC over stdio
  v
approved MCP child process
```

The layers have separate ownership:

- `runtime.rs` owns pending-call order and persistence progression;
- `policy.rs` owns allow, ask, and deny decisions;
- `tool_provider.rs` owns provider catalog, mapping, and dispatch;
- `mcp.rs` owns MCP JSON-RPC and process lifecycle;
- `store.rs` owns attached schemas and persisted outputs.

## Approved Provider Definitions

MCP providers are code-approved, not loaded from arbitrary user config.

| Provider ID | Command | Special environment or cleanup |
| --- | --- | --- |
| `cua-driver` | `cua-driver mcp` | Runs `cua-driver stop` on explicit provider cleanup |
| `desktop-commander` | `npx -y @wonderwhy-er/desktop-commander@0.2.44` | Isolated provider `HOME` and generated config |
| `blender-mcp` | `uvx --python 3.11 blender-mcp==1.6.0` | Telemetry disabled; localhost port `9876` |
| `brightdata` | `npx -y @brightdata/mcp` | Child `API_TOKEN` comes from `BRIGHTDATA_API_TOKEN` |
| `exa` | `npx -y exa-mcp-server@3.2.1` | Child `EXA_API_KEY` comes from Windie's `EXA_API_KEY` environment |

Provider definitions also contain a stable ID, model-facing schema prefix,
display name, optional setup action, and optional shutdown command.

## Environment Resolution

MCP command environment values are typed as:

- a path below Windie's data directory;
- a fixed literal owned by the provider definition;
- a named value copied from Windie's user environment.

Missing required user environment values fail before the child is started.
Windie passes only the explicitly defined provider environment plus the normal
process environment required by command execution.

Exa uses its official, version-pinned stdio package. Store its key in Windie's
canonical provider environment:

```dotenv
EXA_API_KEY=...
```

The default Exa catalog currently includes `web_search_exa` and
`web_fetch_exa`; Windie exposes them with the `exa__` schema prefix. Search is
still performed by Exa's hosted API. The local package is the MCP adapter, not
an offline search engine.

## Desktop Commander Setup

Windie assigns Desktop Commander an isolated home under:

```text
<windie-data>/mcp/desktop-commander
```

Before catalog or execution startup, Windie writes
`.claude-server-commander/config.json` there. The generated config:

- disables telemetry and onboarding;
- preserves a high-risk shell command blocklist;
- limits file write and read line counts;
- sets `allowedDirectories` to an empty list.

In the currently pinned Desktop Commander behavior, that empty list is treated
as allowing every directory. Attachment and Windie approval remain the outer
execution boundaries; there is no Windie filesystem sandbox added here.

## MCP Session Startup

Starting a session performs:

1. spawn the approved command with piped stdin, stdout, and stderr;
2. start background stdout-line and bounded stderr readers;
3. send JSON-RPC `initialize` using protocol version `2025-06-18`;
4. identify the client as `windie` with its package version;
5. wait for the matching initialize response;
6. send `notifications/initialized`.

Only after this handshake can Windie issue `tools/list` or `tools/call`.

## Stdio JSON-RPC

Requests are one-line JSON-RPC 2.0 objects written to child stdin. Every
request receives an incrementing numeric ID. Windie reads stdout lines until it
finds a decoded response with that ID.

While waiting, Windie:

- ignores blank lines;
- ignores lines that are not recognized response objects;
- ignores responses for other IDs;
- turns JSON-RPC error objects into operation errors;
- requires a `result` field on successful responses.

This is a minimal client. It does not currently dispatch server-to-client MCP
requests or consume dynamic tool-list-changed notifications.

## Tool Discovery Lifecycle

`tools/list` always uses a short-lived MCP session:

```text
provider setup
  -> start and initialize
  -> tools/list
  -> decode catalog
  -> drop child session
  -> run provider shutdown hook when configured
```

The provider adapter maps every MCP tool into Windie's shared tool definition.
Successful definitions are cached by the registry, not by `mcp.rs`.

Catalog startup, initialization, timeout, or decoding failures are returned to
the registry. Listing all providers skips unavailable providers; listing one
specific provider surfaces its error.

## Tool Execution Lifecycle

Before MCP execution, the provider adapter validates that:

- the attached provider ID matches this MCP provider;
- the model-called name matches the attached schema name;
- the assistant argument text parses as JSON.

Invalid argument JSON becomes a failed tool output without starting MCP.

The adapter invokes `tools/call` with the provider-native name:

```json
{
  "name": "provider_native_name",
  "arguments": {}
}
```

CLI and API use different session ownership, described below.

## One-Shot Sessions

One-shot CLI execution starts and initializes an MCP process for the call,
sends `tools/call`, then drops the process. If the provider has a shutdown hook,
Windie runs it after the session ends.

This is appropriate for the CLI because every command is already a separate
Windie process.

## Persistent API Sessions

The API creates one registry with an `McpSessionPool`. The pool stores one live
session per provider ID.

On every call, the pool:

1. checks whether the stored command and shutdown definition still match;
2. stops and replaces a mismatched session;
3. starts a session if none exists;
4. updates its last-used time;
5. performs `tools/call` while holding the pool lock;
6. updates last-used time after success.

Calls through the same pool are serialized because session state and the child
stdio channel are protected by one mutex. Different providers are also behind
that pool mutex during a call; the current implementation does not execute two
MCP calls from the same registry in parallel.

## Idle Timeout

Persistent sessions expire after five minutes without a call. A background
reaper wakes every 30 seconds, checks `last_used_at`, removes idle sessions, and
runs configured provider shutdown hooks.

If a persistent request fails, Windie immediately removes that provider's
session and runs its shutdown hook. The next call starts a fresh session.

Dropping the registry kills child processes through `McpSession::drop`, but it
does not run the separate provider shutdown command. Explicit idle, error, or
command-change cleanup does run that hook.

## Request Timeouts

Timeouts are selected by MCP method:

- `tools/call`: five minutes;
- all other protocol requests: 30 seconds.

The timeout covers waiting for each matching response. A timeout carries typed
provider, method, and duration fields.

Approved `tools/call` timeouts become structured failed tool outputs containing
milliseconds and seconds. Protocol and catalog timeouts remain operation errors
because no persisted assistant call is necessarily awaiting a result.

Provider shutdown commands have a separate ten-second process timeout. Windie
retries them up to four times with a 750-millisecond delay. Shutdown is
best-effort and does not turn an already completed user operation into a
failure.

## Process Failure and Diagnostics

Windie captures at most 16 KiB of provider stderr. MCP protocol, child-exit, and
timeout errors include captured stderr when available. A truncation marker is
added when the capture reaches its bound.

Dropping any `McpSession` kills and waits for its child. A disconnected stdout
reader, an MCP error response, or a missing result is treated as failure.

## Result Normalization

The provider adapter converts an MCP `tools/call` result into ordered Windie
message parts:

- `text` blocks become text parts;
- `image` blocks are base64-decoded with their MIME type;
- unsupported block kinds become explanatory text;
- non-null `structuredContent` is appended as text;
- an otherwise empty result falls back to its complete JSON string.

The visible tool-message preview joins text and image summaries. The ordered
parts are persisted separately so image-capable model requests can replay real
image blocks.

MCP's `isError: true` marks the result unsuccessful, but its content is still
normalized and persisted as the required output for the call.

## Persistence Boundary

Live sessions and catalog caches are process-only. The conversation stores the
attached schema and provider mapping. Tool outputs are normal durable message
nodes linked by provider call ID.

Restarting Windie therefore loses MCP processes and caches but not which tools
were attached or what previous calls returned.

## Current Non-Features

- arbitrary user-configured MCP commands;
- MCP over HTTP or WebSocket transports;
- server-to-client request handling;
- dynamic catalog invalidation notifications;
- per-tool or per-directory MCP sandboxing owned by Windie;
- concurrent calls through one API session pool.

## Relevant Code

- `src/mcp.rs`
- `src/tool_provider.rs`
- `src/tool.rs`
- `src/runtime.rs`
