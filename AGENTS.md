# Canonical file guide
- This file is canonical, update this file when the documented information below is missing or outdated.
- /Users/peterbui/windie-Sandbox-workspace/windie.

# Windie Agent Instructions

Before working in this codebase, always read `Backend.md` and `Frontend.md` first.

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

The current goal is to build the cleanest minimal local AI runtime primitives
and a localhost developer API harness for testing those primitives.

Windie talks to Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should only need one OpenAI-compatible query path for now.

Conversation storage is a tree. Runtime execution uses an explicit selected
message head through that tree. Model context is the flattened path to that
head.

Sessions are durable branch objects over the shared conversation tree. The
backend owns session-head resolution: the browser sends a conversation ID and
selected message head, and SQLite determines whether one existing session
matches, no session matches, or the request is ambiguous. Query and continue
routes resolve-or-create the branch in the store and reject stale-head or
ambiguous requests. The frontend displays that result and never infers session
ownership from its cached session list.

## Collaboration Rule

Only give your opinion when asked. Your job is to read code and provide facts. Do not modify codebase unless explicitly told so.

## North Star

The long-term goal is a local AI runtime that lives on the user's computer and can eventually grow into an AI operating layer.

The system should be able to use tools with permission, sandboxed by default, and extended through clean components.

The long-term runtime should support a general wakeup primitive. A wakeup is any event that causes Windie to become active: user input, a schedule, a self-requested continuation, a file event, a browser event, or a system event. Treat chat as one wakeup source, not the whole runtime. Future wakeups should enter through the same path: construct a message, load conversation/context, query the model, and continue only within permission boundaries.

The future direction includes:

- local AI interaction through clean clients
- dynamic conversation/session manipulation such as insert, remove, truncate, forks.
- local tool execution with explicit permission boundaries
- browser-use and computer-use as local capabilities
- user-controlled memory and workspace context
- clear approval policy for risky actions

## Runtime Quality Bar

Windie is a foundational AI sandbox runtime. The codebase should prioritize safety, reliability, clarity, consistency, auditability, and performance.

Prefer typed runtime contracts over loose strings, maps, and ad hoc JSON. Use enums and newtypes for important identifiers, roles, state transitions, wakeups, permissions, tools, provider behavior, and persistence boundaries.

Avoid hidden side effects. Runtime actions should flow through explicit components and clear permission boundaries. Future OS-level capabilities such as tool execution, browser-use, computer-use, file access, wakeups, and memory must be inspectable and controllable.

Engineers should be able to understand, test, and replace each component without reading the whole codebase. If a design becomes hard to explain, treat that as a code smell.

## Engineering Preferences

- Prefer minimal, direct Rust over framework-heavy abstractions.
- Be unbiased and honest in technical discussion. Truth and engineering clarity matter more than agreement or emotional comfort.
- Challenge weak assumptions directly and respectfully when the code, architecture, or product direction would suffer.
- Keep code readable for someone still learning software engineering.
- Always add Rust module docs at the top of every source file using `//!`.
- Always write detailed documentation for meaningful code. Important structs, enums, functions, helpers, and non-obvious logic should have comments that explain their responsibility, data flow, and invariants.
- Prefer typed contracts over raw strings for important runtime concepts.
- Use foundational, direct, clean names for functions, variables, structs, modules, and files.
- Prefer names that state the component's concrete responsibility over clever, vague, or product-shaped names.
- Add abstractions only when they preserve or clarify the component boundaries.
- Avoid adding features just because they are convenient.
- Do not introduce config systems until the current hardcoded path becomes a real limitation.
- Do not reintroduce slash commands unless explicitly requested.
- Do not add agent/tool behavior until explicitly requested.
- Keep dependencies small and justified.

## Architecture

The code should stay split by concrete responsibilities. The detailed backend and frontend maps live in `Backend.md` and `Frontend.md`; read those files first for file-by-file ownership and boundary rules.

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
- Only `dev/` should own local developer helper launchers such as the inspector.
- Only `tool_provider/` should own provider catalog and execution dispatch across code-approved MCP providers and future plugins.
- Only `store/` should own persisted message history, attached tools, and know about SQLite tables and queries.
- Only `store/` and the session operation/manager boundary should resolve or create a session branch for a conversation head; the frontend session cache is presentation state only.
- Only `tool/` should own tool provider, attachment, approval, and execution result data shared across runtime, output, policy, store, and executors.
- `main.rs` should stay small and only wire components together.

Schema compatibility is not a current goal. `store/` should create the current schema for fresh databases and reject unsupported older or newer schema versions clearly instead of carrying partial legacy migrations.
