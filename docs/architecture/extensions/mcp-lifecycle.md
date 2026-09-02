# MCP lifecycle

## Purpose

The MCP lifecycle is how Windie prepares a provider, discovers its tools,
starts a connection when a tool is needed, reuses that connection, and shuts it
down. The lifecycle keeps package preparation and protocol work observable and
bounded before Windie allows the provider to run tool calls.

## Owns

Windie owns the lifecycle for both local stdio MCPs and hosted Streamable HTTP
MCPs.

For a local MCP, Windie can install its declared runtime, prepare the package,
apply its isolated environment and configuration, and start the MCP child
process. For a hosted MCP, Windie creates an HTTP client, applies the declared
authentication, and completes the MCP initialization handshake.

Both transports use the same lifecycle after startup: Windie calls
`tools/list`, stores the provider's discovered tool catalog, and later sends
approved `tools/call` requests through the provider registry.

## Does not own

An MCP session is not a Windie conversation or durable session. The MCP
session is the live protocol connection to one provider. Windie conversation
history, session branches, approvals, and tool results are owned by the
conversation, session, store, and runtime layers.

The MCP lifecycle also does not decide whether a call is allowed. Attachment,
approval, provider health, and component state are checked before execution
enters the MCP provider.

## Main flow

1. Windie validates the installed plugin and MCP component. Setup checks the
   current platform and required secrets, installs declared runtimes, runs any
   package preparation, applies configuration, and moves the provider through
   `updating` while these steps run.
2. Windie starts the local process or connects to the hosted endpoint and
   completes MCP initialization. It calls `tools/list` and stores the result in
   the provider tool catalog. A successful setup records the provider as
   `enabled`; a failed setup records it as `broken` with an actionable error.
3. A health check repeats discovery and any provider-declared readiness probe.
   Repair repeats setup after moving the provider through `updating`.
4. When an approved attached tool is called, the API's persistent MCP session
   pool reuses one live session for that provider across conversations. If no
   session exists, Windie starts or initializes one before sending
   `tools/call`.
5. A provider session records its last use. The API stops sessions that have
   been idle for five minutes. Provider errors also remove the failed session
   so the next call can start a fresh one. CLI invocations use short-lived
   sessions because each CLI command is a separate process.
6. Uninstall stops the provider session before removing the provider runtime
   and store record. Local stdio children are terminated by Windie; hosted HTTP
   sessions receive a best-effort session deletion request.

## Important invariants

- Persistent sessions are keyed by provider ID, not by conversation. A single
  API process therefore does not create a separate MCP process for every
  conversation using the same provider.
- Discovery and tool calls are bounded. Initialization and other protocol
  operations use a 30-second default timeout; `tools/call` uses a five-minute
  default timeout. Hosted MCP endpoints may declare their own non-zero limits.
- A timeout, protocol error, or provider failure does not leave a runtime turn
  waiting forever. Windie turns an execution failure into a failed tool result
  that can be persisted and shown to the model.
- A provider must be installed, enabled, healthy, and attached before its tool
  call can execute. Setup or discovery alone does not attach its schemas to a
  conversation.
- Provider shutdown happens before its runtime or package record is removed.

## Related code

- [`src/operation/component.rs`](../../../src/operation/component.rs) —
  provider setup, health checks, repair, enable/disable, and uninstall.
- [`src/mcp/mcpb.rs`](../../../src/mcp/mcpb.rs) — validates and extracts local
  MCPB runtimes.
- [`src/mcp/stdio.rs`](../../../src/mcp/stdio.rs) — local MCP process startup,
  JSON-RPC calls, timeouts, and process shutdown.
- [`src/mcp/http.rs`](../../../src/mcp/http.rs) — hosted Streamable HTTP
  initialization, requests, authentication, and session shutdown.
- [`src/mcp/session.rs`](../../../src/mcp/session.rs) — provider-keyed session
  reuse and idle cleanup.
- [`src/mcp/loader.rs`](../../../src/mcp/loader.rs) — turns installed MCP
  package components into runtime transports.
- [`src/tool/registry.rs`](../../../src/tool/registry.rs) — provider
  registration, discovery, dispatch, and session stopping.
