# Backend mental model

## Conversation and input

Mental model:

### Conversation

- conversation/id.rs: typed IDs; ConversationId, MessageId, ImageAssetId, and CompactionId.
- conversation/message.rs: Core Message node, Message + Role. Roles are system, user, assistant, tool. System is system prompt message, User is user input message, Assistant is assistant response message, Tool is the tool output message corresponding to assistant response message tool call.
- conversation/assistant_metadata.rs: Assistant message metadata; tool calls, reasoning, audio, annotations, citations, token usage, and refusals. Also includes the tool-call ID that links a tool result to its assistant request.
- conversation/mod.rs: Module boundary and re-exports for conversation types.
- conversation/message_part.rs: Shared ordered text/image parts for persisted messages. User and `role: tool` messages can both carry these parts; message role and assistant-tool-call linkage remain separate.

### Input

- input/: concrete user input loading before conversation storage.
- input/mod.rs: Public boundary and re-exports for input folder.
- input/image.rs: reads and validates user-provided local image files and API-provided image bytes before they are copied into conversation storage.

## LLM

- llm/client.rs: does HTTP to Bifrost.
- llm/mod.rs: Public boundary and re-exports for llm folder.
- llm/model.rs: handle model discovery/parameters.
- llm/responses.rs: provider JSON structs, typed mirror of the provider's Responses API JSON.
- llm/serialization.rs: turn Windie types into provider wire types. Message + ToolSchema -> ResponsesRequest.
- llm/stream.rs: turns provider stream events into Windie assistant events and assembles the final AssistantResponse.
- llm/management.rs: Talks to Bifrost's provider-management API. Lists llm providers, creates provider configurations, submits provider API keys. Does not own inference.
- llm/tests.rs: tests the llm boundary: request serialization, model urls, image handling, tool calls, token counts, and streamed response parsing.

## Store and persistence

- store/mod.rs: Public boundary and re-exports for store folder.
- store/provider.rs: Persist Windie's tool provider lifecycle records in SQLite:
installed, enabled, disabled, broken, or updating, does not install these packages.
- store/provider_tool_catalog.rs: Persists the last discovered MCP tool schemas
  for each installed provider, including fresh, stale, and unavailable status.
- store/compaction.rs: summary checkpoint store, saves and loads compaction checkpoints.
- store/conversation.rs: creates, lists, deletes conversations and stores conversation-level settings like model, reasoning effort, tool approval mode.
- store/message.rs: stores the whole conversation tree. Load paths, insert messages, store messages, including text and image parts, replaces, removes, truncates messages, and forks to another conversation at current message head.
- docs/conversation-tree-and-paths.md: explains why the shared message tree is canonical and why model context resolves a selected root-to-head path instead of storing duplicated linear paths.
- store/schema.rs: database shape, schema version checks, table creation, indexes, and unsupported database version rejection.
- store/session.rs: stores sessions and queued inputs, updates current heads/status, resolves session branches at conversation heads, atomically resolves-or-creates branches, and stores/replays session events.
- store/system_prompt.rs: stores one conversation-wide system prompt, shared by every branch/head.
- store/tool_schema.rs: stores conversation-wide attached tool-schema rows; tools are shared by every branch/head and are not path-filtered.
- store/tests.rs: integration tests for SQLite storage, including conversations, messages, images, tools, sessions, queues, compaction, provider state, branching, deletion and schema safety.

## Operations

- operation/: shared workflow layer between clients and core systems.
- operation/mod.rs: Public boundary and re-exports for operation folder.
- operation/conversation.rs: conversation workflows.
- operation/gateway.rs: gateway/model metadata/input-token workflows.
- operation/input.rs: message input part and image loading workflows.
- operation/inspection.rs: Read-only inspection snapshots for a conversation/head, including tree, selected path, model context, prompt, tools, and compaction.
- operation/message.rs: message and system prompts mutations workflows.
- operation/tool.rs: tool catalog, attachments, mutations workflows.
- operation/session.rs: session lifecycle, backend-owned branch resolution, and runtime advancement workflows.
- operation/session_approval.rs: session-owned tool approval workflows.
- operation/session_cli.rs: CLI adapter over session workflows.
- operation/provider.rs: coordinates provider lifecycle workflows: setup, installation, health checks, enabling, disabling, repairing and uninstalling.
- operation/onboarding.rs: shared onboarding workflow. Configures Bifrost LLM providers, stores MCP secrets, sets up MCP providers, checks provider health, and enables healthy providers.
- operation/tests.rs: tests cross-component workflows built on top of storage, providers, tools, input handling, inspection, and sessions.

## API

- api/: localhost HTTP interface for clients to access Windie runtime primitives.
- api/mod.rs: Public boundary and re-exports for api folder.
- api/router.rs: maps HTTP URLs to API handlers and applies shared request rules.
- api/state.rs: shared API server state passed into route handlers.
- api/error.rs: turns internal Windie errors into HTTP JSON errors.
- api/router.rs: localhost API routes are intentionally unauthenticated and stay bound to the loopback interface.
- api/sse.rs: serializes replayed and live session events for HTTP streaming, hydrating state-changing events with session and message snapshots.
- api/health.rs: API health and runtime status routes.
- api/gateway.rs: model and input-token HTTP routes, plus Bifrost LLM provider catalogs, provider-key management, and provider configuration. It never starts or stops Bifrost.
- api/conversation.rs: conversation-level HTTP routes.
- api/inspection.rs: conversation inspection HTTP route.
- api/message.rs: message and system prompt HTTP routes.
- api/tool.rs: tool catalog, attachment, and tool mutation HTTP routes.
- api/session.rs: session lifecycle, conversation-head resolution/query/continue, and event HTTP routes.
- api/session_approval.rs: session approval HTTP routes.
- api/provider.rs: HTTP handlers for listing and managing Windie tool providers.
- api/env.rs: securely writes manifest-declared provider secrets to ~/.windie/.env and refuses arbitrary environment keys.
- api/shutdown.rs: unauthenticated localhost graceful-stop route used by `windie api stop`; signals api/mod.rs without changing Bifrost.
- api/tests.rs: test HTTP routes, unauthenticated access, error mapping, SSE/session behavior, conversation operating, tools, and mock Bifrost responses.
- config.rs: shared environment-backed gateway, API, and Inspector endpoint configuration.

## CLI

- lib.rs: shared module library consumed by both the public `windie` binary
  and the repository-only `windie-dev` binary.
- cli/: parses terminal arguments into typed CLI commands.
- cli/mod.rs: Public boundary and re-exports for cli folder.
- cli/command.rs: Contract between cli parse and main.rs. Defines parse CLI command types.
- cli/parser.rs: Reads argv and decides which CLI parse should handle it.
- cli/session.rs: Parses session commands, `windie run ...`, etc.
- cli/message.rs: Parses message-related commands, `insert .. message`, `update ... message`, etc.
- cli/tool_schema.rs: Parses tool schema commands, `windie insert <conversation_id> toolschema ... `, etc.
- bin/windie-dev.rs: Repository-only development supervisor, release workflow,
  and benchmark command surface. It is not included in public release archives.
- cli/env.rs: Parses environment variable commands, `windie env KEY=value`, etc.
- cli/onboard.rs: terminal input/output adapter for onboarding. it prompts for provider choices, api keys, mcp secrets, and displays progress
- bin/windie-dev.rs: foreground development supervisor, release workflow
  adapter, and benchmark entry point. It is deliberately excluded from public
  release archives.
- local/process.rs: persistent PID files, detached stdout/stderr logs, and process lifecycle for independent gateway, API, Inspector, and tray components.
- ../vendor/windie-inspector/host/src/main.rs: standalone Inspector static host; its address
  and API endpoint are configurable through the local endpoint environment
  settings. It is an independent Cargo package, not a Windie runtime target.
- tray.rs: simple macOS/Windows tray controller that invokes the lifecycle CLI commands and polls localhost health.
- cli/tests.rs: test cli command parsing and validation

## Tools, builtin tools, skills, plugins, and MCP

- tool/: common tool schema Windie uses for all tool systems.
- tool/mod.rs: Public boundary and re-exports for tool folder.
- tool/approval.rs: Approval data types: approval mode and pending approval request.
- tool/policy/mod.rs: Approval decision rules: allow, ask, or deny a pending tool call.
- tool/policy/tests.rs:
- tool/provider.rs: Provider identity types: typed references from Windie tools to executable backends.
- tool/result.rs: Tool execution result shape, including the `role: tool` message preview, tool-call link, and optional text/image parts. Both builtin and MCP tools use this contract.
- tool/schema.rs: Model-facing tool schema.
- builtin_tools/: Windie-owned control-plane tools. Their schemas are always available to the model and are not persisted as conversation tools.
- builtin_tools/definitions.rs: internal Windie control definitions plus the model-facing `read_skill` and `attach_plugin` definitions.
- builtin_tools/mod.rs: builtin tool lookup, execution, and delegation to the skill, plugin, and MCP registries. Provider-level compatibility operations remain host/runtime operations and are not exposed on the initial model request.
- skills/: reusable, directory-shaped, non-executable instructions for supported workflows.
- skills/manifest.rs: typed skill identity, entrypoint, bounded file list, and model-facing skill document types.
- skills/path.rs: validates bundle-relative skill reference paths and rejects traversal/absolute paths.
- skills/embedded.rs: compile-time embedded asset definitions generated from curated skill directories.
- skills/registry.rs: curated skill catalog and on-demand entrypoint/reference-file loading.
- skills/curated/: bundled first-party Markdown skill directories, including `cua-driver/SKILL.md` and supporting references.
- plugins/: model-facing composition of reusable skills and approved MCP servers. Plugins do not implement transport or execution.
- plugins/curated.rs: trusted code-owned plugin compositions, currently the CUA Driver plugin.
- plugins/manifest.rs: typed plugin metadata and references to skill IDs and MCP server IDs.
- plugins/package.rs: validated file-based plugin package loader and local package cache boundary. Package discovery is separate from MCP process activation.
- plugins/registry.rs: curated plugin catalog, compact runtime catalog prompt, skill loading, and MCP server activation.
- mcp/: MCP protocol and transport boundary.
- mcp/mod.rs: MCP stdio/HTTP client, JSON-RPC handshake, session pooling, tool listing, and tool calls.
- mcp/http.rs: Streamable HTTP MCP transport.
- mcp/registry.rs: MCP server lifecycle, discovery, persisted catalog attachment, and execution dispatch.
- mcp/installation.rs: approved MCP installation targets, provider cleanup, and CUA Driver installation/uninstallation.
- mcp/runtime.rs: Windie-managed Node.js/uv provisioning and executable resolution for MCP servers.
- mcp/manifest.rs: metadata contract for one MCP server's launch command, platform, dependencies, secrets, permissions, scope, and setup information.
- mcp/lifecycle.rs: persisted MCP server installation and readiness states.
- mcp/cleanup.rs: cleanup actions for approved MCP server runtimes.
- mcp/servers/mod.rs: MCP adapter boundary and approved MCP provider re-exports.
- mcp/servers/approved.rs: code-approved MCP provider catalog.
- mcp/servers/blender.rs: Blender MCP definition.
- mcp/servers/brightdata.rs: Brightdata MCP definition.
- mcp/servers/cua.rs: Cua Driver MCP definition.
- mcp/servers/desktop_commander.rs: Desktop Commander MCP definition.
- mcp/servers/basic_memory.rs: Basic Memory MCP definition and isolated local memory project setup.
- mcp/servers/provider.rs: generic MCP backend adapter; lists MCP tools and converts them into Windie ToolDefinitions.
- mcp/servers/executor.rs: executes already-approved MCP tool calls.
- mcp/servers/result.rs: normalizes MCP results into Windie tool output, text, and image message parts.
- store/chrome_devtools.rs: persists the user-selected Chrome DevTools MCP connection mode; connection behavior remains owned by `mcp/`.

## Sessions

- session/: session domain types and live session supervision.
- session/mod.rs: Public boundary and re-exports for session folder.
- session/event.rs: event types for observable session activity. Records events from a running session/agent loop such as streamed assistant text, tool calls, approvals, completion, failure, cancellation, and queued/started inputs.
- session/id.rs: SessionId identifies a durable session; SessionInputId identifies one queued input inside that session.
- session/control.rs: explicit session controls such as cancellation, separate from wakeups that resume runtime work.
- session/manager.rs: manages live background session tasks, approvals, cancellation, and publishes session events.
- session/model.rs: durable session record and lifecycle status. Exists so a session can outlive any one client and can be inspected, resumed, approved, or replayed later.

## Performance

- perf/:
- perf/runtime.rs: builds local sqlite/runtime fixtures and measures runtime operations such as path loading, context building, tool approval, deletion, truncation, and mcp calls.
- perf/mod.rs: Public boundary and re-exports for perf folder.
- perf/mode.rs: benchmark mode, category and option types.
- perf/report.rs: benchmark result data and duration summaries.
- perf/comparison.rs: compared benchmark reports against baseline report.
- perf/runner.rs: benchmark execution entry points.
- perf/fixture.rs: temporary benchmark conversation creation/setup.
- perf/storage.rs: reads and writes benchmark report files.
- perf/tests.rs:

## Developer, local, and output tooling

- local/: user-local Windie environment setup.
- local/mod.rs: Public boundary and re-exports for local folder.
- local/setup.rs: user-local Windie setup, ~/.windie/.env editing, component PID/log paths, and approved dependency installs.
- local/runtime.rs: downloads, verifies, extracts, and resolves managed Node.js/uv runtimes for MCP providers.
- output/:
- output/mod.rs: public boundary and re-exports for output folder
- output/terminal.rs: owns terminal behavior, printing assistant streaming text, tool calls, help, errors, conversations, sessions, models, benchmark reports, and JSON output. It implements the RuntimeOutput interface used by the runtime.
- output/formatting.rs: Converts data into displayable strings or JSON shapes: message previews, trees, help lines, model lists, conversation lists, durations, and performance reports. It contains presentation formatting, not runtime decisions.
- output/tests.rs:

## Runtime and core boundaries

- runtime/:
- runtime/mod.rs: public boundary and re-exports for runtime folder.
- runtime/turn.rs: Runs model turns. Loads the selected conversation head, builds model context, stream assistant response, saves assistant message, and continues through automatic tool calls until completion or approval is needed. 
- runtime/tool_execution.rs: handles tool calls. identifies pending calls, enforeces tool policy, executes approved provider or built-in tools, enforces tool-call order, and save tool results
- runtime/tests.rs:
- main.rs: front desk for the windie binary.
- context.rs: model-facing context finalizer, resolve system prompt, tool schema, messages, compaction summary given one explicit message head.
- mcp.rs: owns the approved MCP stdio protocol boundary: process startup, JSON-RPC initialization, tool listing/calls, timeouts, cleanup hooks, and persistent provider-session pooling.
- gateway.rs: manages the Bifrost LLM gateway.
- wakeup.rs: typed events that resume runtime work, currently session-targeted tool approval decisions. Cancellation is a separate session control.
- error.rs: Typed Windie errors.
- ../vendor/windie-inspector/frontend: local browser client for inspecting and testing Windie through
  the API.

## Runtime behavior and invariants

Conversations are durable message trees:
- insert: add a child message under a selected head.
- rm: remove a node, splicing the tree or deleting tool-call groups when needed.
- truncate: remove descendants after a selected node.
- fork: copy a selected path into a new conversation.
- update: replace node content.
- session/query: run from a selected head and append assistant/tool nodes as results.
- show/tree/inspect: inspect the tree, path, and model-facing context.

Sessions are durable branch objects over a conversation tree. A session stores
the branch's base/current message heads, runtime status, queued inputs,
approvals, and event history; it does not copy messages. The conversation owns
the shared tree and a session owns serialized execution from one selected head.

Session identity is always the durable session ID. The current head is only the
session's position in the conversation tree. Store operations resolve a
conversation/head pair against SQLite and return an existing branch, no branch,
or an explicit ambiguity result. Query and continue requests that target a
conversation head use the store's immediate transaction to resolve-or-create
the branch, then verify that the requested head is still current before
starting execution. Multiple sessions at one current head are rejected as a
conflict rather than selected by order.

Session queries are serialized per session. A query received while the agent
loop is running is stored as a durable FIFO session input rather than inserted
into the conversation tree immediately. When the active run completes, Windie
materializes the oldest queued input under the latest session head and starts
the next run. This keeps queued inputs from becoming stale tree branches and
lets the inspector display queue state without owning execution.

## Architecture

Keep boundaries strict:

- Only `llm/` should know about provider HTTP request details.
- Only `mcp.rs` should know about MCP stdio JSON-RPC request/response details.
- Only `api/` should know about localhost API routes, JSON request bodies, SSE, auth, and HTTP response mapping.
- Only `cli/` should know about startup CLI argument parsing.
- Only `operation/` should own shared CLI/API orchestration over store/runtime primitives. It should not parse argv, map HTTP, format terminal output, execute shell commands, or know provider HTTP details.
- Only `gateway.rs` should know about gateway health/availability/startup checks.
- Only `input/` should know about local user input loading before conversation storage.
- Only `output.rs` should know about terminal and JSON output formatting.
- Only `tool/policy.rs` should decide whether tool execution is allowed, denied, or requires approval.
- Only `conversation/` should own message roles, typed conversation/message identifiers, user parts, model-facing tool schema types, and assistant metadata types.
- Only `session/` should own session domain types, session events, and live session task management.
- Only `context.rs` should decide what history the model sees.
- Only `error.rs` should own typed Windie error categories used across client protocol boundaries.
- Only `perf/` should own benchmark timing logic, reports, comparisons, and benchmark fixture setup.
- Only `runtime.rs` should coordinate query-like runtime flows.
- Only `local/` should own user-local directory setup, `~/.windie/.env` editing, and approved dependency install/check commands.
- `src/bin/windie-dev.rs` owns repository-only development helper launchers and
  `docs/development.md` documents that workflow, while
  `vendor/windie-inspector/` owns the first-party Inspector client and host.
- Only `builtin_tools/` should own Windie control-tool definitions and execution. Only `skills/` should own skill assets and loading. Only `plugins/` should compose skills with MCP server references. Only `mcp/` should own MCP lifecycle, transport, discovery, and execution.
- Only `store/` should own persisted message history, attached tools, and know about SQLite tables and queries.
- Only `store/` and the session operation/manager boundary should resolve or create a session branch for a conversation head; the frontend session cache is presentation state only.
- Only `tool/` should own tool provider, attachment, approval, and execution result data shared across runtime, output, policy, store, and executors.
- `main.rs` should stay small and only wire components together.
