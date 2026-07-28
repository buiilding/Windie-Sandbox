Mental model:
- conversation/id.rs: ids; ConversationID, MessageID, ImageAssetID, CompactionID.
- conversation/message.rs: Core Message node, Message + Role. Roles are system, user, assistant, tool. System is system prompt message, User is user input message, Assistant is assistant response message, Tool is the tool output message corresponding to assistant response message tool call.
- conversation/assistant_metadata.rs: Assitant message Metadata; toolcall, reasoning, assistant audio, assistant annotation, assistant citation, assistant token usage, assistant refusal. Also includes toolcallid to link with tool output message.
- conversation/mod.rs: Module boundary and re-exports for conversation types.
- conversation/user_part.rs: User input message parts, including image part and text part.
- input/: concrete user input loading before conversation storage.
- input/mod.rs: Public boundary and re-exports for input folder.
- input/image.rs: reads and validates user-provided local image files before they are copied into conversation storage.
- llm/client.rs: does HTTP to Bifrost.
- llm/mod.rs: Public boundary and re-exports for llm folder.
- llm/model.rs: handle model discovery/parameters.
- llm/responses.rs: provider JSON structs, typed mirror of the provider's Responses API JSON.
- llm/serialization.rs: turn Windie types into provider wire types. Message + ToolSchema -> ResponsesRequest.
- llm/stream.rs: turn provider stream events into Windie assistant stream. SSE events -> AssitantResponse.
- llm/management.rs: Talks to Bifrost's provider-management API. Lists llm providers, creates provider configurations, submits provider API keys. Does not own inference.
- llm/tests.rs: tests the llm boundary: request serialization, model urls, image handling, tool calls, token counts, and streamed response parsing.
- store/mod.rs: Public boundary and re-exports for store folder.
- store/provider.rs: Persist Windie's tool provider lifecycle records in SQLite:
installed, enabled, disabled, broken, or updating, does not install these packages.
- store/compaction.rs: summary checkpoint store, saves and loads compaction checkpoints.
- store/conversation.rs: creates, lists, deletes conversations and stores conversation-level settings like model, reasoning effort, tool approval mode.
- store/message.rs: stores the whole conversation tree. Load paths, insert messages, store messages, including text and image parts, replaces, removes, truncates messages, and forks to another conversation at current message head.
- store/schema.rs: database shape, schema version checks, table creation, indexes, and unsupported database version rejection.
- store/session.rs: stores sessions, updates current heads/status, resolves session branches at conversation heads, atomically resolves-or-creates branches, and stores/replays session events.
- store/system_prompt.rs: store path-scoped system prompt messages.
- store/tool_schema.rs: store path-scoped tool schema rows.
- store/tests.rs: integration tests for SQLite storage, including conversations, messages, images, tools, sessions, queues, compaction, provider state, branching, deletion and schema safety.
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
- operation/onboarding.rs: shared onboarding workflow. configures bifrost llm providers, stores mcp secrets, sets up mcp providers, check health, enables health.
- operation/tests.rs: tests cross-component workflows built on top of storage, providers, tools, input handling, inspection, and sessions.
- api/: localhost HTTP interface for clients to access Windie runtime primitives.
- api/mod.rs: Public boundary and re-exports for api folder.
- api/router.rs: maps HTTP URLs to API handlers and applies shared request rules.
- api/state.rs: shared API server state passed into route handlers.
- api/error.rs: turns internal Windie errors into HTTP JSON errors.
- api/auth.rs: API token gate before protected routes run.
- api/sse.rs: formats session events for live HTTP streaming.
- api/health.rs: API health and runtime status routes.
- api/gateway.rs: model, gateway, and input-token HTTP routes.
- api/conversation.rs: conversation-level HTTP routes.
- api/inspection.rs: conversation inspection HTTP route.
- api/message.rs: message and system prompt HTTP routes.
- api/tool.rs: tool catalog, attachment, and tool mutation HTTP routes.
- api/session.rs: session lifecycle, conversation-head resolution/query/continue, and event HTTP routes.
- api/session_approval.rs: session approval HTTP routes.
- api/provider.rs: http handlers for listing and managing windie tool providers
- api/env.rs: securely writes declared provider secretes to ~/.windie/.env. refuses arbitrary environment keys
- api/tests.rs: test http routes, authentication, error mapping, sse/session behavior, conversation operating, tools, and mock bifrost responses
- api/ui.rs: serves the compiled browser inspector and its static assets directly from the windie api server.
- cli/: parses terminal arguments into typed CLI commands.
- cli/mod.rs: Public boundary and re-exports for cli folder.
- cli/command.rs: Contract between cli parse and main.rs. Defines parse CLI command types.
- cli/parser.rs: Reads argv and decides which CLI parse should handle it.
- cli/session.rs: Parses session commands, `windie run ...`, etc.
- cli/message.rs: Parses message-related commands, `insert .. message`, `update ... message`, etc.
- cli/tool_schema.rs: Parses tool schema commands, `windie insert <conversation_id> toolschema ... `, etc.
- cli/bench.rs: Parses benchmark commands, `windie bench`, etc.
- cli/env.rs: Parses environment variable commands, `windie env KEY=value`, etc.
- cli/onboard.rs: terminal input/output adapter for onboarding. it prompts for provider choices, api keys, mcp secrets, and displays progress
- cli/tests.rs: test cli command parsing and validation
- tool/: common tool schema Windie uses for all tool systems.
- tool/mod.rs: Public boundary and re-exports for tool folder.
- tool/approval.rs: Approval data types: approval mode and pending approval request.
- tool/policy/mod.rs: Approval decision rules: allow, ask, or deny a pending tool call.
- tool/policy/tests.rs:
- tool/provider.rs: Provider identity types: typed references from Windie tools to executable backends.
- tool/result.rs: Tool output execution result shape.
- tool/schema.rs: Model-facing tool schema.
- tool_provider/: Manages executable tools.
- tool_provider/builtin.rs: defines windie-owned tools that are always visible to the model, currently provider discovering and provider attachment.
- tool_provider/lifecycle.rs: defines the persisted lifecycle states for tool providers
- tool_provider/manifest.rs: defines the metadata contract for a provider: identity, launch command, platform, dependencies, secrets, permissions, scope and setup information.
- tool_provider/mod.rs: Public boundary and re-exports for tool_provider folder.
- tool_provider/registry.rs: The provider-neutral registry, for mcps, builtins, skills, plugins, returns them as available tools. organize and route across catalog families.
- tool_provider/mcp/mod.rs: Public boundary and re-exports for tool_provider/mcp folder.
- tool_provider/mcp/approved.rs: Approved MCP providers for Windie.
- tool_provider/mcp/blender.rs: Blender MCP definition.
- tool_provider/mcp/brightdata.rs: Brightdata MCP definition.
- tool_provider/mcp/cua.rs: Cua Driver MCP definition.
- tool_provider/mcp/desktop_commander.rs: Desktop Commander MCP definition.
- tool_provider/mcp/basic_memory.rs: basic memory mcp provider definition and creates windie's isolated local memory project.
- tool_provider/mcp/provider.rs: Generic MCP backend adapter; list MCP tools, converts them into Windie ToolDefinition.
- tool_provider/mcp/executor.rs: Executes already-approved MCP tool calls.
- tool_provider/mcp/result.rs: MCP result normalization, errors into output, text, image to message parts, build the visible preview stored on the tool message row.
- tool_provider/tests.rs:
- session/: session domain types and live session supervision.
- session/mod.rs: Public boundary and re-exports for session folder.
- session/event.rs: event types for obsrvable session activity. Records observable events from a running session/agent loop such as streamed assistant text, tool calls, approvals, completion, failure, and cancellation.
- session/id.rs: SessionID type for identifying a session.
- session/manager.rs: manages live background session tasks, approvals, cancellation, and publishes session events.
- session/model.rs: durable session record and lifecycle status. Exists so a session can outlive any one client and can be inspected, resumed, approved, or replayed later.
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
- dev/: local developer tooling.
- dev/mod.rs: Public boundary and re-exports for dev folder.
- dev/inspector.rs: launch the local browser inspector UI and passes it the API token.
- local/: user-local Windie environment setup.
- local/mod.rs: Public boundary and re-exports for local folder.
- local/setup.rs: user-local Windie setup, ~/.windie/.env editing, API token storage, and approved dependency installs.
- output/:
- output/mod.rs: public boundary and re-exports for output folder
- output/terminal.rs: owns terminal behavior, printing assistant streaming text, tool calls, help, errors, conversations, sessions, models, benchmark reports, and JSON output. It implements the RuntimeOutput interface used by the runtime.
- output/formatting.rs: Converts data into displayable strings or JSON shapes: message previews, trees, help lines, model lists, conversation lists, durations, and performance reports. It contains presentation formatting, not runtime decisions.
- output/tests.rs:
- runtime/:
- runtime/mod.rs: public boundary and re-exports for runtime folder.
- runtime/turn.rs: Runs model turns. Loads the selected conversation head, builds model context, stream assistant response, saves assistant message, and continues through automatic tool calls until completion or approval is needed. 
- runtime/tool_execution.rs: handles tool calls. identifies pending calls, enforeces tool policy, executes approved provider or built-in tools, enforces tool-call order, and save tool results
- runtime/tests.rs:
- main.rs: front desk for the windie binary.
- context.rs: model-facing context finalizer, resolve system prompt, tool schema, messages, compaction summary given one explicit message head.
- mcp.rs: starts the mcp stdio client.
- gateway.rs: manages the Bifrost LLM gateway.
- wakeup.rs: why the llm is queried.
- error.rs: Typed Windie errors.
- ../dev/windie-inspector: local browser developer UI for inspecting and testing Windie through the API.

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
