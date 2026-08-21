# Backend mental model

## Conversation and input

Mental model:

### Conversation

- conversation/id.rs: typed IDs; ConversationId, MessageId, ImageAssetId, and CompactionId.
- conversation/message.rs: Core Message node, Message + Role. Roles are system, user, assistant, tool. System messages include the user-owned conversation system prompt and generated Windie runtime metadata; User is user input; Assistant is assistant response; Tool is the tool output corresponding to an assistant tool call.
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
- store/component.rs: Persist Windie's installed tool-component lifecycle records in SQLite:
installed, enabled, disabled, broken, or updating, does not install these packages.
- store/tool_catalog.rs: Persists the last discovered MCP tool schemas
  for each installed provider, including fresh, stale, and unavailable status.
- store/compaction.rs: summary checkpoint store, saves and loads compaction checkpoints.
- store/conversation.rs: creates, lists, deletes conversations and stores conversation-level settings like model, reasoning effort, tool approval mode.
- store/message.rs: stores the whole conversation tree. Load paths, insert messages, store messages, including text and image parts, replaces, removes, truncates messages, and forks to another conversation at current message head.
- store/runtime_access.rs: persists the one hosted account explicitly authorized to use this local runtime and prevents another account from replacing it.
- docs/conversation-tree-and-paths.md: explains why the shared message tree is canonical and why model context resolves a selected root-to-head path instead of storing duplicated linear paths.
- store/schema.rs: database shape, schema version checks, table creation, indexes, and unsupported database version rejection.
- store/session.rs: stores sessions and queued inputs, updates current heads/status, resolves session branches at conversation heads, atomically resolves-or-creates branches, and stores/replays session events.
- store/system_prompt.rs: stores one user-owned conversation-wide system prompt, shared by every branch/head. It never stores generated runtime metadata such as the plugin index.
- store/tool_schema.rs: stores conversation-wide attached tool-schema rows; tools are shared by every branch/head and are not path-filtered.
- store/tests.rs: integration tests for SQLite storage, including conversations, messages, images, tools, sessions, queues, compaction, provider state, branching, deletion and schema safety.

## Operations

- operation/: shared workflow layer between clients and core systems.
- operation/mod.rs: Public boundary and re-exports for operation folder.
- operation/conversation.rs: conversation workflows.
- operation/gateway.rs: gateway/model metadata/input-token workflows. Its token preview compiles the exact same model payload as execution without mutating a conversation.
- operation/input.rs: message input part and image loading workflows.
- operation/inspection.rs: Read-only inspection snapshots for a conversation/head, including the durable tree, path, and attached schemas plus the separate final model-context projection and exact ephemeral model schemas, prompt, and compaction.
- operation/message.rs: message and system prompts mutations workflows.
- operation/tool.rs: tool catalog, attachments, mutations workflows.
- operation/session.rs: session lifecycle, backend-owned branch resolution, and runtime advancement workflows.
- operation/session_approval.rs: session-owned tool approval workflows.
- operation/session_cli.rs: CLI adapter over session workflows.
- operation/component.rs: coordinates installed component lifecycle workflows: setup, installation, health checks, enabling, disabling, repairing and uninstalling.
- operation/system.rs: shared lifecycle operations for API, Inspector, and gateway process management.
- operation/onboarding.rs: shared onboarding workflow. Configures Bifrost LLM providers, stores MCP secrets, sets up MCP components, checks component health, and enables healthy components.
- operation/tests.rs: tests cross-component workflows built on top of storage, providers, tools, input handling, inspection, and sessions.

## API

- api/: localhost HTTP interface for clients to access Windie runtime primitives.
- api/mod.rs: Public boundary and re-exports for api folder.
- api/router.rs: maps HTTP URLs to API handlers and applies shared request rules.
- api/state.rs: shared API server state passed into route handlers.
- api/error.rs: turns internal Windie errors into HTTP JSON errors.
- api/runtime_access.rs: validates hosted Supabase account sessions and requires the one explicitly paired account before access to local runtime state.
- api/router.rs: local runtime routes remain loopback-bound and require hosted-account authorization; health and shutdown stay available for local lifecycle checks.
- api/sse.rs: serializes replayed and live session events for HTTP streaming, hydrating state-changing events with session and message snapshots plus the canonical final assistant text on aggregate completion events.
- api/event.rs: exposes the database-wide durable session-event cursor and
  aggregate SSE feed for clients that need to observe durable activity across
  sessions. Typed filters let narrow consumers observe selected event kinds
  without receiving token deltas.
- api/health.rs: API health and runtime status routes.
- api/gateway.rs: model and input-token HTTP routes, plus Bifrost LLM provider catalogs, provider-key management, and provider configuration. It never starts or stops Bifrost.
- api/conversation.rs: conversation-level HTTP routes.
- api/inspection.rs: conversation inspection HTTP route.
- api/message.rs: message and system prompt HTTP routes.
- api/tool.rs: tool catalog, attachment, and tool mutation HTTP routes.
- api/session.rs: session lifecycle, conversation-head resolution/query/continue, and event HTTP routes.
- api/session_approval.rs: session approval HTTP routes.
- api/component.rs: HTTP handlers for listing and managing installed tool components. The current `/api/providers` routes remain backward-compatible.
- api/plugin.rs: marketplace discovery plus plugin install and uninstall routes;
  returns generated presentation summaries alongside versioned releases.
- api/dev.rs: volatile development-only presentation signals, including the
  tray assistant-completed notification probe; it never writes session state.
- api/env.rs: securely writes manifest-declared provider secrets to ~/.windie/.env and refuses arbitrary environment keys.
- api/shutdown.rs: unauthenticated localhost graceful-stop route used by `windie api stop`; signals api/mod.rs without changing Bifrost.
- api/tests.rs: test HTTP routes, hosted-account pairing, error mapping, SSE/session behavior, conversation operating, tools, and mock Bifrost responses.
- config.rs: shared environment-backed gateway, API, Inspector, and hosted-account configuration.

## CLI

- lib.rs: shared module library consumed by the public `windie` binary,
  including its repository development command groups.
- cli/: parses terminal arguments into typed CLI commands and adapts those
  commands to shared runtime operations for the terminal process.
- cli/mod.rs: Public boundary and re-exports for cli folder.
- cli/adapter/mod.rs: Dispatches parsed commands to the domain-specific CLI
  adapters.
- cli/adapter/system.rs: Adapts process lifecycle, gateway, onboarding,
  environment, installation, and help commands.
- cli/adapter/conversation.rs: Adapts conversation creation, inspection,
  listing, branching, deletion, and settings commands; inspection loads the
  same installed capability sources as the API before compiling its snapshot.
- cli/adapter/message.rs: Adapts direct message and system-prompt mutations.
- cli/adapter/tool.rs: Adapts provider-tool and conversation tool-schema
  commands.
- cli/adapter/session.rs: Adapts durable session listing, control, approval,
  and event commands.
- cli/command.rs: Typed contract between CLI parsing and CLI adapters.
- cli/parser.rs: Reads argv and decides which CLI parse should handle it.
- cli/session.rs: Parses session commands, `windie run ...`, etc.
- cli/message.rs: Parses message-related commands, `insert .. message`, `update ... message`, etc.
- cli/tool_schema.rs: Parses tool schema commands, `windie insert <conversation_id> toolschema ... `, etc.
- dev.rs: repository development supervisor, release workflow, local
  marketplace, and benchmark command implementations reached through `windie`.
- cli/env.rs: Parses environment variable commands, `windie env KEY=value`, etc.
- cli/onboard.rs: terminal input/output adapter for onboarding. it prompts for provider choices, api keys, mcp secrets, and displays progress
- dev.rs: foreground development supervisor, release workflow adapter, local
  marketplace, and benchmark entry point dispatched by the public CLI.
- local/process.rs: persistent PID files, detached stdout/stderr logs, and process lifecycle for independent gateway, API, Inspector, tray, and notifier components.
- ../vendor/windie-inspector/host/src/main.rs: standalone Inspector static host; its address
  and API endpoint are configurable through the local endpoint environment
  settings. Its loopback `POST /shutdown` route lets the Inspector stop itself
  gracefully. It is an independent Cargo package, not a Windie runtime target.
- local/tray.rs: macOS/Windows tray presentation component that polls local component health and requests explicit single-component lifecycle operations.
- local/notifier.rs: independent notification process that starts durable completion and development-probe observers without owning a tray or runtime service.
- local/session_event_observer.rs: reconnecting aggregate session-completion SSE observer that persists the last displayed cursor and forwards a preview of only canonical final durable assistant responses to the notifier.
- local/tray_notification.rs: native notification presenter plus the development-only notification SSE probe. Platform click actions open the durable session's hosted Inspector URL; the probe never touches durable session state.
- cli/tests.rs: test cli command parsing and validation

## Tools and providers

- tool/: common tool schema Windie uses for all tool systems.
- tool/mod.rs: Public boundary and re-exports for tool folder.
- tool/approval.rs: Approval data types: approval mode and pending approval request.
- tool/policy/mod.rs: Approval decision rules: allow, ask, or deny a pending tool call.
- tool/policy/tests.rs:
- tool/provider.rs: Provider identity types: typed references from Windie tools to executable backends.
- tool/builtin.rs: defines Windie-owned control tools that `runtime/context.rs`
  adds to every final model payload; they are not persisted as conversation tools.
- tool/lifecycle.rs: defines persisted lifecycle states for installed tool components.
- tool/manifest.rs: runtime-facing provider metadata projected from package manifests.
- tool/registry.rs: provider-neutral live discovery and execution dispatch. It projects installed MCP components into Windie's model-facing tool registry.
- tool/result.rs: Tool execution result shape, including the `role: tool` message preview, tool-call link, and optional text/image parts.
- tool/schema.rs: Model-facing tool schema.
- packages/brightdata/: package-owned Bright Data MCPB plugin fixture.
- packages/cua-driver/: package-owned CUA Driver MCPB plugin fixture.
- mcp/: MCP protocol and component runtime boundary.
- mcp/mod.rs: MCP JSON-RPC protocol, stdio and Streamable HTTP session coordination.
- mcp/http.rs: Streamable HTTP transport for remote MCP servers.
- mcp/mcpb.rs: MCPB validation, extraction, and package-owned process command preparation.
- mcp/tool_provider.rs: generic MCP adapter that discovers MCP tools and dispatches approved calls.
- mcp/executor.rs: executes already-approved MCP tool calls.
- mcp/result.rs: normalizes MCP results into Windie tool messages and image parts.
- tool/tests.rs: tool registry, MCP mapping, and result normalization tests.

## Sessions

- session/: session domain types and live session supervision.
- session/mod.rs: Public boundary and re-exports for session folder.
- session/event.rs: event types for observable session activity. Records events from a running session/agent loop such as streamed assistant text, tool calls, approvals, completion, failure, cancellation, and queued/started inputs.
- session/id.rs: SessionId identifies a durable session; SessionInputId identifies one queued input inside that session; SessionExecutionClaimId is the unique fencing token for one execution attempt.
- session/control.rs: explicit session controls such as cancellation, separate from wakeups that resume runtime work.
- session/manager.rs: manages live background session tasks, approvals, cancellation, and publishes session events.
- session/model.rs: durable session record, lifecycle status, execution-owner kind, and unique execution claim. Exists so a session can outlive any one client and can be inspected, resumed, approved, or replayed later.

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
- local/setup.rs: user-local Windie setup, ~/.windie/.env editing, component PID/log paths, temporary compatibility dependency installs, and exact release-owned desktop-bundle removal during uninstall.
- local/process.rs: local Windie process lifecycle, PID files, logs, and shutdown.
- local/tray.rs: local desktop tray lifecycle and health polling.
- local/notifier.rs: local cross-platform notification lifecycle and observer ownership.
- local/session_event_observer.rs: durable final-session-completion observer with a persisted notifier cursor and native-text preview boundary.
- local/tray_notification.rs: native notification presenter and local development probe observer.
- managed_runtime/: downloads, verifies, extracts, and resolves managed Node.js/uv runtimes for packaged MCP components.
- managed_runtime/mod.rs: owns shared managed runtime installation and executable resolution.
- output/:
- output/mod.rs: public boundary and re-exports for output folder
- output/terminal.rs: owns terminal behavior, printing assistant streaming text, tool calls, help, errors, conversations, sessions, models, benchmark reports, and JSON output. It implements the RuntimeOutput interface used by the runtime.
- output/formatting.rs: Converts data into displayable strings or JSON shapes: message previews, trees, help lines, model lists, conversation lists, durations, and performance reports. It contains presentation formatting, not runtime decisions.
- output/tests.rs:

## Runtime and core boundaries

- runtime/:
- runtime/mod.rs: public boundary and re-exports for runtime folder.
- runtime/context.rs: the single read-only compiler for the exact model payload
  at one selected head. It combines generated plugin-index capability metadata,
  the user-owned conversation system prompt, selected path, compaction, attached
  schemas, and built-in tool schemas without persisting ephemeral capabilities.
  The plugin index is regenerated for every request and cannot be overwritten by
  changing or clearing the conversation system prompt.
- runtime/turn.rs: Runs model turns. It selects a runnable head, asks
  `runtime/context.rs` for the final model payload unchanged, streams the
  assistant response, saves it, and continues through automatic tool calls
  until completion or approval is needed.
- runtime/tool_execution.rs: handles tool calls. identifies pending calls, enforeces tool policy, executes approved provider or built-in tools, enforces tool-call order, and save tool results
- runtime/wakeup.rs: typed events that resume runtime activity, currently session-targeted tool approval decisions.
- runtime/tests.rs:
- main.rs: front desk for the windie binary.
- llm/gateway.rs: manages the local Bifrost LLM gateway lifecycle and health checks.
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
- show/tree/inspect: inspect the tree, path, final model-context projection,
  and runtime schema list.

`runtime/context.rs` is the only module that decides model-visible messages
and schemas. `runtime/turn.rs` owns execution workflow, and
`runtime/tool_execution.rs` owns tool-policy decisions and result persistence;
neither may add or remove model context after it is compiled. API inspection
and input-token counting call the same context compiler, so their output shows
the runtime capabilities that execution uses rather than a persisted-only
approximation.

Sessions are durable branch objects over a conversation tree. A session stores
the branch's base/current message heads, runtime status, queued inputs,
approvals, and event history; it does not copy messages. The conversation owns
the shared tree and a session owns serialized execution from one selected head.
Every execution attempt receives a fresh SQLite-backed claim ID. All streamed
events, assistant/tool-result writes, and terminal transitions must present
that exact fencing token, so an older runner cannot write through a newer API
or CLI claim merely because both have the same owner kind. API and CLI both
enter execution through `execute_session`, and both acquire claims through the
same typed store function; only their output presentation differs.

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

Session event row IDs are monotonic across the whole SQLite database. The
session-specific SSE route uses them as a per-session replay cursor, while the
aggregate `/api/events` route uses the same IDs to merge durable events from
every API or CLI session runner.

## Architecture

Keep boundaries strict:

- Only `llm/` should know about provider HTTP request details.
- Only `mcp/` should know about MCP stdio JSON-RPC request/response details.
- Only `api/` should know about localhost API routes, JSON request bodies, SSE, auth, and HTTP response mapping.
- Only `cli/` should know about startup CLI argument parsing.
- Only `operation/` should own shared CLI/API orchestration over store/runtime primitives. It should not parse argv, map HTTP, format terminal output, execute shell commands, or know provider HTTP details.
- Only `llm/gateway.rs` should know about gateway health/availability/startup checks.
- Only `input/` should know about local user input loading before conversation storage.
- Only `output/` should know about terminal and JSON output formatting.
- Only `tool/policy/` should decide whether tool execution is allowed, denied, or requires approval.
- Only `conversation/` should own message roles, typed conversation/message identifiers, user parts, model-facing tool schema types, and assistant metadata types.
- Only `session/` should own session domain types, session events, and live session task management.
- Only `runtime/context.rs` should decide what history the model sees.
- Only `error.rs` should own typed Windie error categories used across client protocol boundaries.
- Only `perf/` should own benchmark timing logic, reports, comparisons, and benchmark fixture setup.
- Only `runtime/` should coordinate query-like runtime flows.
- Only `local/` should own user-local directory setup, `~/.windie/.env` editing, and local Windie process/tray management.
- Only `managed_runtime/` should install and resolve Windie-managed Node.js and uv runtimes for packaged components.
- Only `dev.rs` should own repository development helper launchers, while
  `vendor/windie-inspector/` owns the first-party Inspector client and host.
- Only `tool/` should own the model-facing provider registry and provider lifecycle projection.
- Only `mcp/` should own MCP protocol, transport, MCPB, MCP tool discovery, and MCP result adaptation.
- Only `store/` should own persisted message history, attached tools, and know about SQLite tables and queries.
- Only `store/` and the session operation/manager boundary should resolve or create a session branch for a conversation head; the frontend session cache is presentation state only.
- Only `tool/` should own tool provider, attachment, approval, and execution result data shared across runtime, output, policy, store, and executors.
- `main.rs` should stay small and only wire components together.

## Complete `src/` file inventory

This is the canonical one-row-per-file index for the Rust runtime. The
sections above explain the architecture; this index makes the complete source
surface auditable when files are added or moved.

### Root modules

- `src/config.rs`: shared local endpoint configuration.
- `src/dev.rs`: repository development, release, marketplace, and benchmark workflows.
- `src/error.rs`: typed Windie errors shared across client boundaries.
- `src/lib.rs`: shared Windie runtime library exported to the binary and tests.
- `src/main.rs`: public Windie CLI entrypoint and dependency wiring.

### Local API (`src/api/`)

- `src/api/component.rs`: installed tool-component lifecycle API handlers.
- `src/api/conversation.rs`: conversation-level API handlers.
- `src/api/env.rs`: manifest-declared provider-secret environment API handlers.
- `src/api/error.rs`: JSON error mapping for the localhost API boundary.
- `src/api/event.rs`: aggregate runtime-event HTTP routes.
- `src/api/gateway.rs`: Bifrost gateway, model, and input-token API handlers.
- `src/api/health.rs`: health and runtime-status API handlers.
- `src/api/inspection.rs`: conversation-inspection API handlers.
- `src/api/message.rs`: message and system-prompt API handlers.
- `src/api/mod.rs`: local API server boundary and startup.
- `src/api/plugin.rs`: marketplace plugin API handlers.
- `src/api/router.rs`: local API route table and HTTP middleware wiring.
- `src/api/runtime_access.rs`: hosted-account validation and local runtime pairing handlers/middleware.
- `src/api/session.rs`: session lifecycle and event API route handlers.
- `src/api/session_approval.rs`: session-approval API route handlers.
- `src/api/shutdown.rs`: local API graceful-shutdown handling.
- `src/api/sse.rs`: server-sent-event helpers for streaming session events.
- `src/api/state.rs`: shared API server state and store-access helpers.
- `src/api/tests.rs`: API route tests.
- `src/api/tool.rs`: tool-catalog and tree-wide tool-mutation API handlers.

### CLI (`src/cli/`)

- `src/cli/adapter/conversation.rs`: terminal adapters for conversation creation, inspection, and settings.
- `src/cli/adapter/message.rs`: terminal adapters for direct conversation-message mutations.
- `src/cli/adapter/mod.rs`: dispatches parsed commands to domain-specific CLI adapters.
- `src/cli/adapter/session.rs`: terminal adapters for durable session commands.
- `src/cli/adapter/system.rs`: terminal adapters for process, gateway, onboarding, and environment commands.
- `src/cli/adapter/tool.rs`: terminal adapters for provider-tool and conversation tool-schema commands.
- `src/cli/command.rs`: typed CLI command data.
- `src/cli/development.rs`: parser for repository development, release, marketplace, and benchmark commands.
- `src/cli/env.rs`: provider-key environment command parsing.
- `src/cli/message.rs`: message command parsing.
- `src/cli/mod.rs`: Windie CLI parsing boundary.
- `src/cli/onboard.rs`: terminal prompts for `windie onboard`.
- `src/cli/parser.rs`: top-level argv dispatch for the CLI parser.
- `src/cli/session.rs`: session command parsing.
- `src/cli/tests.rs`: CLI parser tests.
- `src/cli/tool_schema.rs`: tool-schema command parsing.

### Conversation and input (`src/conversation/`, `src/input/`)

- `src/conversation/assistant_metadata.rs`: assistant-oriented metadata lanes.
- `src/conversation/id.rs`: typed conversation, message, image-asset, and compaction identifiers.
- `src/conversation/message.rs`: conversation message data and roles.
- `src/conversation/message_part.rs`: shared text and image parts for persisted conversation messages.
- `src/conversation/mod.rs`: core conversation data boundary.
- `src/input/image.rs`: local image-input loading and validation.
- `src/input/mod.rs`: local user-input loading boundary.

### LLM and local process support (`src/llm/`, `src/local/`, `src/managed_runtime/`)

- `src/llm/client.rs`: Bifrost Responses HTTP client.
- `src/llm/error.rs`: typed failures returned by the provider-facing LLM boundary.
- `src/llm/gateway.rs`: Bifrost gateway availability and lifecycle.
- `src/llm/management.rs`: Bifrost management API client.
- `src/llm/mod.rs`: OpenAI-compatible Bifrost client boundary.
- `src/llm/model.rs`: Bifrost model identity, model listing, and model-parameter metadata.
- `src/llm/responses.rs`: OpenAI-compatible Responses wire structs.
- `src/llm/serialization.rs`: conversion from Windie messages and tools into Responses wire values.
- `src/llm/stream.rs`: Responses stream parsing and assistant-response assembly.
- `src/llm/tests.rs`: Bifrost client-boundary tests.
- `src/local/mod.rs`: user-local Windie environment boundary.
- `src/local/process.rs`: detached local component process management.
- `src/local/setup.rs`: user-local setup, environment editing, and approved dependency installation.
- `src/local/tray.rs`: desktop tray controller for the Windie runtime.
- `src/managed_runtime/mod.rs`: Windie-managed runtime provisioning and executable resolution.

### MCP and plugin packages (`src/mcp/`, `src/plugin/`)

- `src/mcp/chrome_devtools.rs`: persisted connection-mode types for the package-owned Chrome DevTools MCP.
- `src/mcp/executor.rs`: approved MCP tool executor.
- `src/mcp/http.rs`: Streamable HTTP MCP client.
- `src/mcp/loader.rs`: MCP component loading from installed plugin packages.
- `src/mcp/mcpb.rs`: MCPB package validation and extraction.
- `src/mcp/mod.rs`: MCP protocol, transport, and runtime boundary.
- `src/mcp/protocol.rs`: MCP protocol contracts and JSON-RPC data shapes.
- `src/mcp/result.rs`: MCP tool-call result normalization.
- `src/mcp/session.rs`: persistent MCP session lifecycle.
- `src/mcp/stdio.rs`: local stdio MCP transport.
- `src/mcp/tool_provider.rs`: generic MCP tool-provider adapter.
- `src/mcp/transport.rs`: MCP transport routing.
- `src/plugin/catalog.rs`: marketplace index contracts and model-facing plugin summaries.
- `src/plugin/installer.rs`: marketplace artifact acquisition and verification.
- `src/plugin/manifest.rs`: typed plugin and component-manifest contracts.
- `src/plugin/mod.rs`: installable Windie plugin-package boundary.
- `src/plugin/store.rs`: Windie-owned plugin-package storage.

### Shared operations (`src/operation/`)

- `src/operation/component.rs`: installed tool-component lifecycle operations.
- `src/operation/conversation.rs`: conversation-level operation workflows.
- `src/operation/gateway.rs`: gateway, model-metadata, and input-token workflows.
- `src/operation/input.rs`: operation-level user-input loading helpers.
- `src/operation/inspection.rs`: read-only snapshots for CLI JSON, API, and developer inspection.
- `src/operation/message.rs`: message and system-prompt mutation workflows.
- `src/operation/mod.rs`: shared CLI/API operation-layer boundary.
- `src/operation/onboarding.rs`: shared terminal onboarding workflow.
- `src/operation/session.rs`: runtime session lifecycle and advancement workflows.
- `src/operation/session_approval.rs`: session tool-approval workflows.
- `src/operation/session_cli.rs`: CLI session-operation adapter.
- `src/operation/system.rs`: lifecycle operations for independently managed local components.
- `src/operation/tests.rs`: operation-workflow tests.
- `src/operation/tool.rs`: tool-catalog, attachment, and tool-schema workflows.

### Output and performance (`src/output/`, `src/perf/`)

- `src/output/formatting.rs`: terminal and JSON output-formatting helpers.
- `src/output/mod.rs`: terminal-output boundary.
- `src/output/terminal.rs`: terminal-output implementation.
- `src/output/tests.rs`: terminal-output formatting tests.
- `src/perf/comparison.rs`: benchmark-report comparison.
- `src/perf/fixture.rs`: fixture-construction helpers for local benchmarks.
- `src/perf/mod.rs`: performance measurement and comparison boundary.
- `src/perf/mode.rs`: benchmark mode, category, and option types.
- `src/perf/report.rs`: benchmark-report data and duration summarization.
- `src/perf/runner.rs`: benchmark-runner entry points.
- `src/perf/runtime.rs`: deterministic runtime benchmark fixtures.
- `src/perf/scenarios.rs`: named benchmark scenarios for the current architecture.
- `src/perf/storage.rs`: benchmark-report file storage.
- `src/perf/tests.rs`: performance-report tests.

### Runtime and sessions (`src/runtime/`, `src/session/`)

- `src/runtime/context.rs`: exact model-facing context construction.
- `src/runtime/mod.rs`: runtime-flow coordination boundary.
- `src/runtime/retry.rs`: bounded retry policy for one model turn.
- `src/runtime/tests.rs`: runtime-flow coordination tests.
- `src/runtime/tool_execution.rs`: runtime tool execution and durable result persistence.
- `src/runtime/turn.rs`: runtime turn orchestration.
- `src/runtime/wakeup.rs`: typed wakeup inputs.
- `src/session/control.rs`: explicit control inputs for durable sessions.
- `src/session/event.rs`: replayable session-event types.
- `src/session/id.rs`: session, queued-input, and execution-claim identifiers.
- `src/session/manager.rs`: live session supervision.
- `src/session/mod.rs`: session-domain boundary.
- `src/session/model.rs`: durable session row and lifecycle-status types.

### SQLite store and tool domain (`src/store/`, `src/tool/`)

- `src/store/chrome_devtools.rs`: persisted Chrome DevTools MCP connection settings.
- `src/store/compaction.rs`: conversation-compaction checkpoint persistence.
- `src/store/component.rs`: persisted installed component state.
- `src/store/conversation.rs`: conversation-row persistence and conversation-level settings.
- `src/store/message.rs`: message-tree, message-part, image-asset, and fork persistence.
- `src/store/mod.rs`: SQLite persistence boundary.
- `src/store/runtime_access.rs`: durable hosted-account ownership for the local runtime.
- `src/store/schema.rs`: SQLite schema creation and version validation.
- `src/store/session.rs`: runtime-session and replayable session-event persistence.
- `src/store/system_prompt.rs`: tree-wide user-owned system-prompt persistence.
- `src/store/tests.rs`: SQLite persistence-boundary tests.
- `src/store/tool_catalog.rs`: persisted provider-owned MCP tool catalogs.
- `src/store/tool_schema.rs`: tree-wide tool-capability persistence.
- `src/tool/approval.rs`: tool-approval contracts.
- `src/tool/builtin.rs`: Windie-owned model-control tools.
- `src/tool/lifecycle.rs`: persisted lifecycle states for installed Windie providers.
- `src/tool/manifest.rs`: typed metadata describing an installable Windie provider.
- `src/tool/mod.rs`: tool-domain boundary.
- `src/tool/policy/mod.rs`: tool-execution policy boundary.
- `src/tool/policy/tests.rs`: tool-execution policy-decision tests.
- `src/tool/provider.rs`: tool-provider identity types.
- `src/tool/registry.rs`: provider-neutral tool registry.
- `src/tool/result.rs`: tool-execution result type.
- `src/tool/schema.rs`: model-facing tool-schema and conversation-exposure types.
- `src/tool/tests.rs`: tool-provider catalog, MCP mapping, and result-normalization tests.
