# Windie Agent Instructions

## Project Intent

Windie is the foundational implementation of an AI runtime for the operating
system.

The purpose of this codebase is to build the lower-level runtime that lets AI
operate on a user's computer reliably, safely, quickly, and consistently.
Windie should become the foundation for AI that can live inside the local
operating environment, understand runtime state, act through explicit
permission boundaries, and eventually behave in a proactive, computer-native
way.

Build one clean primitive at a time. Keep the foundation small, fast,
inspectable, and hackable.
The whole codebase should reflect this file.

The current goal is to
build the cleanest minimal local AI runtime primitives and a localhost
developer API harness for testing those primitives:

Windie talks to Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should only need one OpenAI-compatible query path for now.

Conversation storage is a tree. Runtime execution uses an explicit selected
message head through that tree. Model context is the flattened path to that
head.

## Collaboration Rule

Only give your opinion when asked. Your job is to read code and provide facts. Do not modify codebase unless explicitly told so.

## North Star

The long-term goal is a local AI runtime that
lives on the user's computer and can eventually grow into an AI operating
layer.

The system should be able to use tools with permission, sandboxed by default, and extended
through clean components.

The long-term runtime should support a general wakeup primitive. A wakeup is any
event that causes Windie to become active: user input, a schedule, a
self-requested continuation, a file event, a browser event, or a system event.
Treat chat as one wakeup source, not the whole runtime. Future wakeups should
enter through the same path: construct a message, load conversation/context,
query the model, and continue only within permission boundaries.

The future direction includes:

- local AI interaction through clean clients
- dynamic conversation/session manipulation such as insert, remove, truncate, forks.
- local tool execution with explicit permission boundaries
- browser-use and computer-use as local capabilities
- user-controlled memory and workspace context
- clear approval policy for risky actions

## Runtime Quality Bar

Windie is a foundational AI sandbox runtime. The
codebase should prioritize safety, reliability, clarity, consistency,
auditability, and performance.

Prefer typed runtime contracts over loose strings, maps, and ad hoc JSON. Use
enums and newtypes for important identifiers, roles, state transitions, wakeups,
permissions, tools, provider behavior, and persistence boundaries.

Avoid hidden side effects. Runtime actions should flow through explicit
components and clear permission boundaries. Future OS-level capabilities such as
tool execution, browser-use, computer-use, file access, wakeups, and memory must
be inspectable and controllable.

Engineers should be able to understand, test, and replace each component without
reading the whole codebase. If a design becomes hard to explain, treat that as a
code smell.

## Architecture

The code should stay split by concrete responsibilities:

Mental model:
- conversation/id.rs: ids; ConversationID, MessageID, ImageAssetID, CompactionID.
- conversation/message.rs: Core Message node, Message + Role. Roles are system, user, assistant, tool. System is system prompt message, User is user input message, Assistant is assistant response message, Tool is the tool output message corresponding to assistant response message tool call.
- conversation/assistant_metadata.rs: Assitant message Metadata; toolcall, reasoning, assistant audio, assistant annotation, assistant citation, assistant token usage, assistant refusal. Also includes toolcallid to link with tool output message.
- conversation/mods.rs: Module boundary and re-exports for conversation types.
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
- store/mod.rs: Public boundary and re-exports for store folder.
- store/compaction.rs: summary checkpoint store, saves and loads compaction checkpoints.
- store/conversation.rs: creates, lists, deletes conversations and stores conversation-level settings like model, reasoning effort, tool approval mode.
- store/message.rs: stores the whole conversation tree. Load paths, insert messages, store messages, including text and image parts, replaces, removes, truncates messages, and forks to another conversation at current message head.
- store/schema.rs: database shape, schema version checks, table creation, indexes, and unsupported database version rejection.
- store/session.rs: stores sessions, update current heads/status, store/replays session events.
- store/system_prompt.rs: store tree-wide system prompt (conversations.system_prompt column).
- store/tool_schema.rs: store tree-wide tool schema rows (conversation_id, name) primary key.
- operation/: shared workflow layer between clients and core systems.
- operation/mod.rs: Public boundary and re-exports for operation folder.
- operation/conversation.rs: conversation workflows.
- operation/gateway.rs: gateway/model metadata/input-token workflows.
- operation/input.rs: message input part and image loading workflows.
- operation/inspection.rs: Read-only inspection snapshots for a conversation/head, including tree, selected path, model context, prompt, tools, and compaction.
- operation/message.rs: message and system prompts mutations workflows.
- operation/tool.rs: tool catalog, attachments, mutations workflows.
- operation/session.rs: session lifecycle and runtime advancement workflows.
- operation/session_approval.rs: session-owned tool approval workflows.
- operation/session_cli.rs: CLI adapter over session workflows.
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
- api/session.rs: session lifecycle and event HTTP routes.
- api/session_approval.rs: session approval HTTP routes.
- cli/: parses terminal arguments into typed CLI commands.
- cli/mod.rs: Public boundary and re-exports for cli folder.
- cli/command.rs: Contract between cli parse and main.rs. Defines parse CLI command types.
- cli/parser.rs: Reads argv and decides which CLI parse should handle it.
- cli/session.rs: Parses session commands, `windie run ...`, etc.
- cli/message.rs: Parses message-related commands, `insert .. message`, `update ... message`, etc.
- cli/tool_schema.rs: Parses tool schema commands, `windie insert <conversation_id> toolschema ... `, etc.
- cli/bench.rs: Parses benchmark commands, `windie bench`, etc.
- cli/env.rs: Parses environment variable commands, `windie env KEY=value`, etc.
- tool/: common tool schema Windie uses for all tool systems.
- tool/mod.rs: Public boundary and re-exports for tool folder.
- tool/approval.rs: Approval data types: approval mode and pending approval request.
- tool/policy.rs: Approval decision rules: allow, ask, or deny a pending tool call.
- tool/provider.rs: Provider identity types: typed references from Windie tools to executable backends.
- tool/result.rs: Tool output execution result shape.
- tool/schema.rs: Model-facing tool schema.
- tool_provider/: Manages executable tools.
- tool_provider/mod.rs: Public boundary and re-exports for tool_provider folder.
- tool_provider/registry.rs: The provider-neutral registry, for mcps, builtins, skills, plugins, returns them as available tools. organize and route across catalog families.
- tool_provider/mcp/mod.rs: Public boundary and re-exports for tool_provider/mcp folder.
- tool_provider/mcp/approved.rs: Approved MCP providers for Windie.
- tool_provider/mcp/blender.rs: Blender MCP definition.
- tool_provider/mcp/brightdata.rs: Brightdata MCP definition.
- tool_provider/mcp/cua.rs: Cua Driver MCP definition.
- tool_provider/mcp/desktop_commander.rs: Desktop Commander MCP definition.
- tool_provider/mcp/provider.rs: Generic MCP backend adapter; list MCP tools, converts them into Windie ToolDefinition.
- tool_provider/mcp/executor.rs: Executes already-approved MCP tool calls.
- tool_provider/mcp/result.rs: MCP result normalization, errors into output, text, image to message parts, build the visible preview stored on the tool message row.
- session/: session domain types and live session supervision.
- session/mod.rs: Public boundary and re-exports for session folder.
- session/event.rs: event types for obsrvable session activity. Records observable events from a running session/agent loop such as streamed assistant text, tool calls, approvals, completion, failure, and cancellation.
- session/id.rs: SessionID type for identifying a session.
- session/manager.rs: manages live background session tasks, approvals, cancellation, and publishes session events.
- session/model.rs: durable session record and lifecycle status. Exists so a session can outlive any one client and can be inspected, resumed, approved, or replayed later.
- perf/:
- perf/mod.rs: Public boundary and re-exports for perf folder.
- perf/mode.rs: benchmark mode, category and option types.
- perf/report.rs: benchmark result data and duration summaries.
- perf/comparison.rs: compared benchmark reports against baseline report.
- perf/runner.rs: benchmark execution entry points.
- perf/fixture.rs: temporary benchmark conversation creation/setup.
- perf/storage.rs: reads and writes benchmark report files.
- dev/: local developer tooling.
- dev/mod.rs: Public boundary and re-exports for dev folder.
- dev/inspector.rs: launch the local browser inspector UI and passes it the API token.
- local/: user-local Windie environment setup.
- local/mod.rs: Public boundary and re-exports for local folder.
- local/setup.rs: user-local Windie setup, ~/.windie/.env editing, API token storage, and approved dependency installs.
- main.rs: front desk for the windie binary.
- context.rs: model-facing context finalizer, resolve system prompt, tool schema, messages, compaction summary given one explicit message head.
- runtime.rs: simple input/output engine for the LLM. query the path, llm returns the message.
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

Keep boundaries strict:

- Only `llm/` should know about provider HTTP request details.
- Only `mcp.rs` should know about MCP stdio JSON-RPC request/response details.
- Only `api/` should know about localhost API routes, JSON request bodies, SSE, auth, and HTTP response mapping.
- Only `cli/` should know about startup CLI argument parsing.
- Only `operation/` should own shared CLI/API orchestration over store/runtime
  primitives. It should not parse argv, map HTTP, format terminal output,
  execute shell commands, or know provider HTTP details.
- Only `gateway.rs` should know about gateway health/availability/startup checks.
- Only `input/` should know about local user input loading before conversation storage.
- Only `output.rs` should know about terminal and JSON output formatting.
- Only `tool/policy.rs` should decide whether tool execution is allowed, denied,
  or requires approval.
- Only `conversation/` should own message roles, typed conversation/message
  identifiers, user parts, model-facing tool schema types, and assistant metadata
  types.
- Only `session/` should own session domain types, session events, and live
  session task management.
- Only `context.rs` should decide what history the model sees.
- Only `error.rs` should own typed Windie error categories used across client
  protocol boundaries.
- Only `perf/` should own benchmark timing logic, reports, comparisons, and
  benchmark fixture setup.
- Only `runtime.rs` should coordinate query-like runtime flows.
- Only `local/` should own user-local directory setup, `~/.windie/.env`
  editing, and approved dependency install/check commands.
- Only `dev/` should own local developer helper launchers such as the inspector.
- Only `tool_provider/` should own provider catalog and execution dispatch
  across code-approved MCP providers and future plugins.
- Only `store/` should own persisted message history, attached tools, and
  know about SQLite tables and queries.
- Only `tool/` should own tool provider, attachment, approval, and execution
  result data shared across runtime, output, policy, store, and executors.
- `main.rs` should stay small and only wire components together.

Schema compatibility is not a current goal. `store/` should create the
current schema for fresh databases and reject unsupported older or newer schema
versions clearly instead of carrying partial legacy migrations.


## Engineering Preferences

- Prefer minimal, direct Rust over framework-heavy abstractions.
- Be unbiased and honest in technical discussion. Truth and engineering clarity
  matter more than agreement or emotional comfort.
- Challenge weak assumptions directly and respectfully when the code,
  architecture, or product direction would suffer.
- Keep code readable for someone still learning software engineering.
- Always add Rust module docs at the top of every source file using `//!`.
- Always write detailed documentation for meaningful code. Important structs,
  enums, functions, helpers, and non-obvious logic should have comments that
  explain their responsibility, data flow, and invariants.
- Prefer typed contracts over raw strings for important runtime concepts.
- Use foundational, direct, clean names for functions, variables, structs, modules, and files.
- Prefer names that state the component's concrete responsibility over clever, vague, or product-shaped names.
- Add abstractions only when they preserve or clarify the component boundaries.
- Avoid adding features just because they are convenient.
- Do not introduce config systems until the current hardcoded path becomes a real limitation.
- Do not reintroduce slash commands unless explicitly requested.
- Do not add agent/tool behavior until explicitly requested.
- Keep dependencies small and justified.
