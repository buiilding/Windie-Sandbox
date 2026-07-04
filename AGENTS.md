# Windie Agent Instructions

## Project Intent

Windie is a Rust-first, CLI-first local AI querying foundation.

Windie is a ground-up rebuild of the local AI computer-runtime idea. The older
WindieOS explored the broad product vision: AI present inside the user's
desktop session, with access to files, shell, browser-use, computer-use,
memory, permissions, tools, and workflow context. AIOS/agentd explored a
related ownership lesson: durable runtime state, execution, and user interfaces
must have clear boundaries.

This project keeps that long-term direction but intentionally restarts from a
smaller, faster, more hackable foundation. Do not copy old WindieOS complexity
into this codebase. Build one clean primitive at a time.
The whole codebase should reflect this file.

The current goal is not to build a full agent or a TUI. The current goal is to
build the cleanest minimal local AI runtime primitives:

```text
explicit CLI command
-> persisted conversation state
-> model-facing context when needed
-> OpenAI-compatible LLM request when needed
-> persisted result
-> terminal output
```

Windie talks to Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should only need one OpenAI-compatible query path for now.

## North Star

Windie is not a generic chatbot. The long-term goal is a local AI runtime that
lives on the user's computer and can eventually grow into an AI operating
layer.

The system should feel like an AI presence inside the computer: aware of local
context, able to use tools with permission, sandboxed by default, and extended
through clean components.

The long-term runtime should support a general wakeup primitive. A wakeup is any
event that causes Windie to become active: user input, a schedule, a
self-requested continuation, a file event, a browser event, or a system event.
Treat chat as one wakeup source, not the whole runtime. Future wakeups should
enter through the same path: construct a message, load conversation/context,
query the model, and continue only within permission boundaries.

The future direction includes:

- local terminal-first AI interaction
- conversation/session editing, resend, continuation, and forking
- local shell execution with explicit permission boundaries
- browser-use and computer-use as local capabilities
- user-controlled memory and workspace context
- clear approval policy for risky actions
- hackable components that can be inspected, replaced, and extended

Do not implement the whole AI OS early. Build the small primitives that could
support it later: chat, persistence, context, tools, permissions, process
execution, memory, and UI surfaces. The long-term product can grow toward the
old WindieOS idea of AI living inside the user's computing environment, but this
implementation must stay performance-first and architecture-first. Prefer the
smallest lower-level primitive that is correct, tested, and fast.

## Runtime Quality Bar

Windie is a foundational AI sandbox runtime, not a prototype chatbot. The
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

Use the ownership lesson from the older workspaces:

```text
Windie owns local interaction, conversation/runtime flow, and future local tools.
Bifrost owns provider inference, model routing, provider keys, and LLM observability. Reason: Bifrost proves itself to be the fastest, lightest provider adapter.
Clients own user interface surfaces such as CLI, desktop app, browser UI, or voice.
```

The current CLI is the first client and the first runtime surface. It is not the
whole product.

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
- Message append, update, and remove primitives.
- Conversation truncate and fork primitives.
- One-shot conversation query primitive.
- Model-facing context construction.
- Future-ready compaction storage.
- Basic performance baseline measurement.
- OpenAI-compatible `/chat/completions` requests.
- Bifrost gateway health check.
- Explicit Bifrost gateway start and stop commands.
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
- Config files.
- Slash commands.
- Automatic history compaction.
- Memory systems beyond persisted conversation messages and future compaction checkpoints.
- Killing Bifrost automatically on Windie exit.

The CLI should be boring, explicit, and composable. Bare `windie` must not start
chat, create conversations, query models, or mutate persisted state. It should
exit successfully without runtime action. Future TUI, desktop, browser, voice,
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
- Prefer typed contracts over raw strings for important runtime concepts.
- Use foundational, direct, clean names for functions, variables, structs, modules, and files.
- Prefer names that state the component's concrete responsibility over clever, vague, or product-shaped names.
- Add abstractions only when they preserve or clarify the component boundaries.
- Avoid adding features just because they are convenient.
- Do not introduce config systems until the current hardcoded path becomes a real limitation.
- Do not reintroduce slash commands unless explicitly requested.
- Do not add agent/tool behavior until explicitly requested.
- Keep dependencies small and justified.

## Runtime Behavior

The current expected behavior is command-driven:

```text
parse explicit command
load or mutate persisted conversation state
build model context only for query-like commands
stream model response only for query-like commands
save assistant response only after successful inference
print command output
exit
```

Keep primitive operations separate. Put concrete CLI command inventory in
`commands.md`, not in this file.

Query-like commands and live benchmark commands must not silently start Bifrost.
They require the gateway to already be running and should tell the user how to
start the gateway if it is not.

Do not add convenience wrappers that mix update, remove, query, and append into
one unclear command. If a future convenience wrapper is needed, add it only
after the primitive commands are correct.

If a model request fails, let the error stop the program. Do not silently
continue after failed inference.

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
