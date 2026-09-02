# MCP

## Purpose

MCP is a common protocol for people to develop capabilities for AI agents. In
Windie, an MCP component is a provider that exposes tools through this
protocol. The provider can be a local process that Windie starts, or a hosted
MCP service that Windie connects to.

MCP is the boundary between Windie and the provider that implements a tool.
Windie discovers the provider's tools, gives the applicable tool schemas to
the LLM, and sends approved tool calls to the provider.

## Owns

The MCP layer owns the protocol transport, tool discovery, tool calls, timeout
handling, and result normalization. It supports local stdio processes and
hosted Streamable HTTP providers.

An MCP process receives a `tools/call` request, runs the requested operation,
and returns the tool output to Windie. Windie converts that output into its
tool-result shape so the runtime can save it as a durable `role: tool`
message.

## Does not own

MCP does not own plugin packaging, installation, or the local component state.
MCP components are parts of plugins, while the plugin store and component
lifecycle manage their package files and enabled state.

MCP also does not decide whether a tool call is allowed. Windie's tool policy
and conversation approval mode make that decision before the MCP executor is
called. An MCP provider may run code or connect to an external service, so it
can be security-sensitive and computationally heavy; Windie's process and
permission boundaries still apply.

## Main flow

1. Windie loads an MCP component from an installed plugin and registers its
   provider in the tool registry.
2. During setup or an explicit refresh, Windie starts or connects to the MCP
   provider and calls `tools/list`. The discovered schemas are stored in the
   provider's tool catalog.
3. Windie does not attach every MCP schema to every model request. A
   conversation receives the schemas for MCP tools that are attached to that
   conversation, along with Windie's built-in control schemas.
4. The model can call `windie__attach_mcp` for an installed, enabled, healthy
   MCP. Windie stores the discovered schemas, and the MCP tools become
   available on the next model turn after the `attach_mcp` tool result.
5. When the model requests an MCP tool, Windie checks that the tool is
   attached, available, and approved. It sends `tools/call` to the provider,
   waits for the bounded response, normalizes the output, saves the tool
   result, and continues the turn.

## Lifecycle

Before an MCP can run, Windie checks the provider's platform and required
secrets, installs any declared runtime, prepares the package, applies its
configuration, and discovers its tools. A successful setup records the
provider as `enabled`; a failed setup records it as `broken` with an error.
Health checks repeat tool discovery and any readiness probe, while repair runs
the setup phases again.

When an approved tool is first called in the API process, Windie starts or
connects to the provider and keeps the live session in a provider-keyed pool.
Conversations using the same provider reuse that session instead of starting a
separate process for each conversation. The API removes sessions after five
minutes without use or after a provider error. CLI invocations use short-lived
sessions because each CLI command is a separate process.

Uninstall stops the provider session before removing its runtime and store
record. Local stdio children are terminated by Windie, while hosted HTTP
sessions receive a best-effort session deletion request.

## Important invariants

- Only attached schemas for enabled, healthy providers are exposed to a
  conversation and eligible for execution. MCP schemas are not automatically
  added just because a plugin is installed.
- Windie bounds MCP requests: protocol operations such as initialization and
  `tools/list` use a 30-second default timeout, while `tools/call` uses a
  five-minute default timeout. Hosted MCP manifests may declare their own
  non-zero endpoint limits.
- An approved MCP result is normalized and persisted as an ordinary durable
  tool message. A timeout or provider error becomes a failed tool result that
  the model can see; it does not leave the session waiting forever.
- Installing or attaching an MCP does not remove Windie's approval policy.
  The provider receives a tool call only after the runtime has allowed it.

## Related code

- [`src/mcp/protocol.rs`](../../../src/mcp/protocol.rs) — MCP transports,
  schemas, request types, and timeout contracts.
- [`src/mcp/session.rs`](../../../src/mcp/session.rs) — provider-keyed
  persistent sessions and idle cleanup.
- [`src/mcp/tool_provider.rs`](../../../src/mcp/tool_provider.rs) — MCP
  discovery and provider definitions.
- [`src/mcp/executor.rs`](../../../src/mcp/executor.rs) — approved MCP calls
  and result normalization.
- [`src/tool/registry.rs`](../../../src/tool/registry.rs) — provider
  registration and dispatch.
- [`src/runtime/tool_execution.rs`](../../../src/runtime/tool_execution.rs) —
  attachment, approval, and `attach_mcp` behavior.
