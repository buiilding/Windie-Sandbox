# Extensions

This overview explains how plugins, MCP components, skills, app connectors,
and the marketplace fit together. Detailed pages cover each extension type and
its lifecycle.

- [Plugins](plugins.md)
- [MCP](mcp.md) and [MCP lifecycle](mcp-lifecycle.md)
- [Skills](skills.md)
- [App connectors](app-connectors.md)
- [Marketplace](marketplace.md)

## Purpose

Plugins let Windie add optional capabilities without hard-coding each integration
into the runtime. An installed plugin can provide one or more MCP servers,
skills, or app connectors. MCP is the protocol layer used by an MCP component
to describe and run tools.

This keeps the core runtime small while giving the model a controlled way to
discover and use extra local and hosted capabilities.

## Owns

The plugin system owns package identity, versioned installation, manifest
validation, marketplace discovery, and the model-facing index of installed
capabilities.

The MCP layer owns the transport to an MCP server, tool discovery, conversion
of MCP tools into Windie tool schemas, execution of tool calls, and lifecycle
of reusable MCP sessions.

## Does not own

Plugins do not decide whether the model may run a tool. Windie's tool policy
and session-owned approval flow make that decision, and the runtime persists
the resulting tool output in the conversation tree.

Plugins also do not own model context by themselves. `runtime/context.rs`
builds the final request sent to the LLM, including the ephemeral plugin index
and the tool schemas currently attached to the conversation.

## Main flow

1. Windie reads the marketplace index and installed plugin packages. A plugin
   manifest describes its identity, presentation metadata, and components.
2. During MCP component setup or repair, Windie prepares the component and
   asks its MCP server for `tools/list`. The resulting tool names,
   descriptions, and input schemas are stored as that provider's catalog.
3. Each model request receives a compact index of installed plugins. This is
   discovery metadata, not every installed tool's full schema.
4. When the model needs a plugin capability, it can read a listed skill with
   `windie__read_skill` or attach one of the plugin's MCP components with
   `windie__attach_mcp`. Attaching uses the discovered catalog and makes that
   MCP's schemas available on the next model turn.
5. If the model calls an attached MCP tool and the call is approved, Windie
   opens or reuses a session for that MCP provider, sends `tools/call`, saves
   the result as a tool message, and continues the normal runtime turn.

For a local stdio MCP, Windie starts and communicates with a child process.
For a hosted Streamable HTTP MCP, Windie connects to a remote service instead;
it owns the client session, not the remote server process.

```text
Plugin package contents
├── MCP component ──> discover tools ──> stored provider catalog
├── skill ──────────> instructions available to read on demand
└── app connector ──> connector metadata

Model and runtime flow
installed plugins + component metadata ──> compact plugin index in model context
                                                │
                          model may read a skill or attach one MCP
                                                │
                                                v
                               approved tools/call → saved tool result
```

## Important invariants

- A plugin is the installable package boundary. It may contain MCP, skill, and
  app components; it is not itself automatically an executable tool.
- MCP tool schemas are discovered ahead of use, but they are not all exposed
  to the model at once. Only explicitly attached MCP schemas join the next
  model request.
- The plugin index is a generated system message, and `read_skill` and
  `attach_mcp` are generated tool schemas. Windie adds them to each model
  request without saving them in the conversation. They are runtime-supplied
  capability controls, not user-authored conversation content: they let the
  model discover installed plugins and deliberately load instructions or
  attach tools when needed.
- Persistent MCP sessions are keyed by provider, not conversation. The API's
  long-lived registry reuses an idle session and stops it after five minutes
  without use. A CLI invocation uses short-lived MCP execution instead.
- Local stdio MCPs use a 30-second timeout for initialization and tool
  discovery, and a five-minute timeout for tool calls. A hosted Streamable HTTP
  MCP can set a different non-zero startup or call timeout in its manifest; the
  configured value replaces the default rather than being capped at five
  minutes. A tool-call timeout is converted into a failed tool result that the
  model can see; it does not leave the turn waiting forever.
- Package setup, process startup, external access, and tool execution remain
  subject to declared permissions, component state, and Windie's approval
  policy.

## Packages and marketplace

`packages/<plugin-id>/` is the source layout for bundled plugin packages.
Each package has a top-level `plugin.json`; MCP components additionally point
to their MCP `server.json` and may include a local MCPB artifact. The current
repository packages each contain an MCP component, while the manifest format
also supports skills and app connectors.

Packages opt into publication with `"marketplace": { "publish": true }`.
The marketplace workflow builds versioned archives, uploads those archives to
GitHub Releases, and deploys the small catalog site. A normal Windie API uses
`https://marketplace.windieos.com/index.json` as its marketplace index.

## Related code

- `src/plugin/manifest.rs`: typed plugin, MCP, skill, and app manifest
  contracts.
- `src/plugin/store.rs` and `src/plugin/installer.rs`: verified package
  installation and versioned local storage.
- `src/plugin/catalog.rs`: marketplace and installed-plugin summaries,
  including the compact model-facing index.
- `src/mcp/loader.rs`, `src/mcp/tool_provider.rs`, and `src/mcp/session.rs`:
  MCP component loading, tool discovery, transport, and persistent sessions.
- `src/tool/builtin.rs`: `windie__read_skill` and `windie__attach_mcp`.
- `src/runtime/context.rs`: final model context, including ephemeral plugin
  metadata and attached tool schemas.
- `src/dev.rs`: marketplace archive build and publication workflow.
