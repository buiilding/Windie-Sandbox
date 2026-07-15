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

## Ownership Boundaries

```text
Windie owns local interaction, conversation/runtime flow, and future local tools.
Bifrost owns provider inference, model routing, provider keys, and LLM observability. Reason: Bifrost proves itself to be the fastest, lightest provider adapter.
Clients own user interface surfaces such as CLI, desktop app, browser UI, or voice.
```

For multimodal input, Windie owns local file reading, durable copied image
storage, typed message parts, and OpenAI-compatible request shape. Bifrost and
the provider own model capability rejection.

The current CLI is the first client and the first runtime surface. It is not the
whole product.

For each provider Windie wants Bifrost to use, Bifrost needs provider config
once. The provider row names the provider, such as `anthropic`. The key row
points to the environment variable, such as `env.ANTHROPIC_API_KEY`. Use the
same pattern for Gemini, Groq, OpenRouter, and other providers. The actual
secret value should stay in Windie's explicit `~/.windie/.env` provider-key file.

## Current Scope

Build only the foundational local CLI runtime primitives and the localhost
developer API needed to test them.

Allowed in the current scope:

- Rust CLI binary.
- Localhost-only Rust API server for developer test harnesses.
- Localhost developer frontend under `dev/` for testing runtime primitives
  through the API.
- Hardcoded default endpoint/model while the foundation is still forming.
- Explicit primitive CLI commands.
- Streaming assistant output.
- Typed conversation and message data model.
- SQLite-backed conversation persistence.
- Multiple persisted conversations.
- Message insert, update, and remove primitives.
- User image input with copied local image bytes.
- Root-scoped and explicit-head system prompt primitive.
- Conversation truncate and fork primitives.
- Explicit-head path inspection and full conversation tree inspection.
- Read-only JSON inspection for developer tools.
- JSON API access to the same explicit runtime/store primitives as the CLI.
- One-shot conversation query primitive.
- Conversation-level persisted model selection with optional per-query override
  for explicit one-shot calls.
- Tool-call receiving and persistence.
- Conversation-level attached tool persistence and model request serialization.
- Typed assistant metadata lanes for tool calls, reasoning, refusal, audio, and
  annotations.
- Unified tool provider layer for code-approved MCP providers and future
  plugins.
- Code-approved MCP provider tools through the same attached-tool and approval
  boundary.
- Conversation-level manual or full-access approval mode for attached executable
  tools.
- Tool result persistence as `role: tool` messages linked to provider tool-call
  IDs.
- Model-facing context construction.
- Future-ready compaction storage.
- Basic local performance baselines, repeated benchmark runs, and JSON
  benchmark comparison.
- User-local benchmark baseline update and comparison under `~/.windie`.
- OpenAI-compatible `/responses` requests.
- OpenAI-compatible Responses image request serialization.
- Bifrost gateway health check.
- Explicit Bifrost gateway start and stop commands.
- Public Bifrost gateway startup through `npx @maximhq/bifrost` or Docker.
- Explicit `~/.windie/.env` provider-key environment for Bifrost gateway startup.
- Explicit `windie env` editing for Windie's `~/.windie/.env` provider-key file.
- Explicit `windie install` for code-approved public runtime dependencies.
- Clean module boundaries.

Not in scope yet:

- TUI.
- Desktop UI.
- Production browser UI.
- Voice UI.
- Open-ended autonomous agent loops outside explicit query, approval, and
  full-access primitives.
- Browser use.
- Production/general computer use outside code-approved developer MCP providers.
- Plugin systems.
- Production web dashboard.
- General config files beyond the explicit Bifrost `~/.windie/.env` provider-key file.
- Global model selection.
- Slash commands.
- Automatic history compaction.
- Memory systems beyond persisted conversation messages and future compaction checkpoints.
- Killing Bifrost automatically on Windie exit.
- User-configurable arbitrary MCP servers.

The CLI should be boring, explicit, and composable. Future TUI, desktop,
browser, voice, SDK, background worker, plugin, and wakeup clients should
converge through the same shared operation/runtime path to the same primitives.

The `dev/windie-inspector` frontend is a localhost developer client for testing
and inspecting runtime primitives. It may call the API, render runtime state,
and exercise explicit store/runtime operations. It must not own provider logic,
persistence, model context construction, runtime state transitions, tool
execution, or permission policy.

## Architecture

The code should stay split by concrete responsibilities:

```text
src/main.rs          wires components together
src/api.rs           localhost developer API server
src/cli.rs           startup CLI arguments
src/operation.rs     shared CLI/API operation orchestration
src/output.rs        terminal and JSON output only
src/output_tests.rs  terminal output tests
src/policy.rs        tool execution policy and approval boundary
src/policy_tests.rs  tool execution policy tests
src/conversation.rs  message types, model-facing tool schema types, and assistant metadata types
src/context.rs       model-facing context construction
src/error.rs         typed user-facing Windie error categories
src/gateway.rs       Bifrost gateway availability and lifecycle
src/image_input.rs   local image file loading
src/llm.rs           Bifrost/OpenAI-compatible HTTP client
src/mcp.rs           MCP stdio JSON-RPC client and session pool
src/perf.rs          performance baseline measurement
src/runtime.rs       one-shot runtime query coordination
src/runtime_tests.rs runtime flow tests
src/setup.rs         user-local setup, env-file editing, and approved installs
src/tool.rs          tool provider, attachment, approval, and execution result types
src/tool_provider.rs tool provider registry and dispatch
src/store.rs         SQLite persistence
src/store_tests.rs   SQLite persistence tests
```

Keep boundaries strict:

- Only `llm.rs` should know about provider HTTP request details.
- Only `mcp.rs` should know about MCP stdio JSON-RPC request/response details.
- Only `api.rs` should know about localhost API routes, JSON request bodies, and HTTP response mapping.
- Only `cli.rs` should know about startup CLI argument handling.
- Only `operation.rs` should own shared CLI/API orchestration over store/runtime
  primitives. It should not parse argv, map HTTP, format terminal output,
  execute shell commands, or know provider HTTP details.
- Only `gateway.rs` should know about gateway health/availability/startup checks.
- Only `image_input.rs` should know about local image file loading.
- Only `output.rs` should know about terminal and JSON output formatting.
- Only `policy.rs` should decide whether tool execution is allowed, denied, or
  requires approval.
- Only `conversation.rs` should own message roles, typed conversation/message
  identifiers, model-facing tool schema types, and assistant metadata types.
- Only `context.rs` should decide what history the model sees.
- Only `error.rs` should own typed Windie error categories used across client
  protocol boundaries.
- Only `perf.rs` should own benchmark timing logic.
- Only `runtime.rs` should coordinate query-like runtime flows.
- Only `setup.rs` should own user-local directory setup, `~/.windie/.env`
  editing, and approved dependency install/check commands.
- Only `tool_provider.rs` should own provider catalog and execution dispatch
  across code-approved MCP providers and future plugins.
- Only `store.rs` should own persisted message history, attached tools, and
  know about SQLite tables and queries.
- Only `tool.rs` should own tool provider, attachment, approval, and execution
  result data shared across runtime, output, policy, store, and executors.
- `main.rs` should stay small and only wire components together.

Schema compatibility is not a current goal. `store.rs` should create the
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
