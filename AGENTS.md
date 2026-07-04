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
build the cleanest minimal local AI runtime primitives:

Windie talks to Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should only need one OpenAI-compatible query path for now.

Conversation storage is a tree. Runtime execution uses one selected active path
through that tree. Model context is the flattened active path.

## Collaboration Rule

Act primarily as a language-to-code converter for this project. Translate the
user's requested behavior into code, tests, and documentation while keeping the
implementation consistent with this file.

The user makes every product, architecture, naming, command, and feature
decision. Do not make those decisions independently. Present facts, current code
state, consequences, and implementation options when useful, but wait for the
user's decision before changing direction.

Only provide an engineering opinion when the user asks for one. Only object to
the user's command or opinion when fully confident it contradicts this file or
would break a stated project invariant.

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
- local shell execution with explicit permission boundaries
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
shell execution, browser-use, computer-use, file access, wakeups, and memory must
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
secret value should stay in Windie's explicit `.env` provider-key file.

## Current Scope

Build only the foundational local CLI runtime primitives.

Allowed in the current scope:

- Rust CLI binary.
- Hardcoded default endpoint/model while the foundation is still forming.
- Explicit primitive CLI commands.
- Streaming assistant output.
- Typed conversation and message data model.
- SQLite-backed conversation persistence.
- Multiple persisted conversations.
- Message insert, update, and remove primitives.
- User image input with copied local image bytes.
- Conversation-level system prompt primitive.
- Conversation truncate and fork primitives.
- Active path selection and full conversation tree inspection.
- One-shot conversation query primitive.
- Per-query model override with Bifrost-qualified model names.
- Tool-call receiving and persistence without tool execution.
- Model-facing context construction.
- Future-ready compaction storage.
- Basic local performance baselines, repeated benchmark runs, and JSON
  benchmark comparison.
- OpenAI-compatible `/chat/completions` requests.
- OpenAI-compatible image request serialization.
- Bifrost gateway health check.
- Explicit Bifrost gateway start and stop commands.
- Explicit `.env` provider-key environment for Bifrost gateway startup.
- Clean module boundaries.

Not in scope yet:

- TUI.
- Desktop UI.
- Browser UI.
- Voice UI.
- Agentic tool use.
- Shell command execution.
- Browser use.
- Computer use.
- Approval flows.
- Plugin systems.
- Web dashboard.
- General config files beyond the explicit Bifrost `.env` provider-key file.
- Persisted conversation/global model selection.
- Slash commands.
- Automatic history compaction.
- Memory systems beyond persisted conversation messages and future compaction checkpoints.
- Killing Bifrost automatically on Windie exit.

The CLI should be boring, explicit, and composable. Future TUI, desktop, browser, voice,
and wakeup clients should call the same runtime and store primitives that the
CLI uses.

## Architecture

The code should stay split by concrete responsibilities:

```text
src/main.rs          wires components together
src/cli.rs           startup CLI arguments
src/output.rs        terminal output only
src/output_tests.rs  terminal output tests
src/conversation.rs  message types and in-memory conversation state
src/context.rs       model-facing context construction
src/gateway.rs       Bifrost gateway availability and lifecycle
src/image_input.rs   local image file loading
src/llm.rs           Bifrost/OpenAI-compatible HTTP client
src/perf.rs          performance baseline measurement
src/runtime.rs       one-shot runtime query coordination
src/runtime_tests.rs runtime flow tests
src/store.rs         SQLite persistence
src/store_tests.rs   SQLite persistence tests
```

Keep boundaries strict:

- Only `llm.rs` should know about HTTP details.
- Only `cli.rs` should know about startup CLI argument handling.
- Only `gateway.rs` should know about gateway health/availability/startup checks.
- Only `image_input.rs` should know about local image file loading.
- Only `output.rs` should know about printing.
- Only `conversation.rs` should own message roles and typed conversation/message identifiers.
- Only `context.rs` should decide what history the model sees.
- Only `perf.rs` should own benchmark timing logic.
- Only `runtime.rs` should coordinate query-like runtime flows.
- Only `store.rs` should own persisted message history and know about SQLite tables and queries.
- `main.rs` should stay small and only wire components together.

## Teaching Requirement

After creating or modifying project files, explain the change so the user can
learn the codebase instead of only receiving a completion summary.

For each meaningful code change, include:

- what changed and why
- which files were touched
- what each new or changed function/struct is responsible for
- how data flows through the changed code
- what behavior changed for the user, if any
- what tests or checks were run

For each commit made, provide an explicit description of that commit. The
description should state what changed, why it changed, and which behavior or
code boundary the commit affects.

Keep the explanation direct and concrete. Prefer teaching the real code in front
of us over abstract software-engineering vocabulary. The goal is that the user
can gradually understand Windie well enough to navigate and modify it.

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

## Verification

After code changes, run:

```bash
cargo fmt
cargo check
scripts/check.sh
```

`scripts/check.sh` runs the test suite, builds the release binary, and checks
`windie --version` / `windie --help` against the local release binary. It is a
local/free smoke check and should not call Bifrost or a model provider.

Benchmark behavior must keep side effects explicit. Document concrete benchmark
commands in `commands.md`.
