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

The current goal is not to build a full agent. The current goal is to build the cleanest minimal continuous chat/querying engine:

```text
terminal input
-> conversation state
-> OpenAI-compatible LLM request
-> assistant response
-> terminal output
```

Windie talks to Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should only need one OpenAI-compatible query path for now.

## Long-Term Vision

Windie should become a local AI operating layer for the user's computer.

The future direction includes:

- local terminal-first AI interaction
- conversation/session editing, resend, continuation, and forking
- local shell execution with explicit permission boundaries
- browser-use and computer-use as local capabilities
- user-controlled memory and workspace context
- clear approval policy for risky actions
- hackable components that can be inspected, replaced, and extended

The long-term product can grow toward the old WindieOS idea of AI living inside
the user's computing environment, but this implementation must stay
performance-first and architecture-first. Prefer the smallest lower-level
primitive that is correct, tested, and fast.

## Ownership Boundaries

Use the ownership lesson from the older workspaces:

```text
Windie owns local interaction, conversation/runtime flow, and future local tools.
Bifrost owns provider inference, model routing, provider keys, and LLM observability.
Clients own user interface surfaces such as CLI, desktop app, browser UI, or voice.
```

Do not duplicate Bifrost provider infrastructure inside Windie unless Bifrost
becomes a verified bottleneck or wrong dependency. Do not make UI code own
runtime state. Do not make future tools bypass explicit runtime and permission
boundaries.

The current CLI is the first client and the first runtime surface. It is not the
whole product.

## Lessons From Earlier Work

- The vision is AI present on the user's machine, not another generic chatbot.
- The old broad implementation became too complex and slow for this restart.
- This codebase exists to rebuild from experience with sharper boundaries,
  better performance discipline, and less premature platform architecture.
- Avoid demos that bypass the real runtime path.
- Avoid dashboards, plugins, config layers, memory systems, and tool frameworks
  before the lower-level primitive earns them.
- When in doubt, preserve hackability: direct code, obvious data flow, small
  components, and tests around the behavior we own.

## Current Scope

Build only the foundational chat loop.

Allowed in the current scope:

- Rust CLI binary.
- Hardcoded default endpoint/model while the foundation is still forming.
- Continuous terminal chat.
- Streaming assistant output.
- In-memory conversation history.
- OpenAI-compatible `/chat/completions` requests.
- Bifrost gateway health check.
- Bifrost auto-start from the local workspace when it is not already running.
- Clean module boundaries.

Not in scope yet:

- Agentic tool use.
- Shell command execution.
- Browser use.
- Computer use.
- Approval flows.
- Plugin systems.
- Web dashboard.
- Persistence or databases.
- Config files.
- Slash commands.
- Memory systems beyond in-memory session messages.
- Killing Bifrost on Windie exit.

## Architecture

The code should stay split by concrete responsibilities:

```text
src/main.rs          wires components together
src/cli.rs           startup CLI arguments
src/input.rs         terminal input only
src/output.rs        terminal output only
src/conversation.rs  message types and in-memory history
src/gateway.rs       Bifrost gateway availability and startup
src/llm.rs           Bifrost/OpenAI-compatible HTTP client
src/runtime.rs       continuous chat loop
```

Keep boundaries strict:

- Only `llm.rs` should know about HTTP details.
- Only `cli.rs` should know about startup CLI argument handling.
- Only `gateway.rs` should know about gateway health/availability/startup checks.
- Only `input.rs` should know about stdin reading.
- Only `output.rs` should know about printing.
- Only `conversation.rs` should own message history.
- Only `runtime.rs` should coordinate the loop.
- `main.rs` should stay small and only wire components together.

## Engineering Preferences

- Prefer minimal, direct Rust over framework-heavy abstractions.
- Keep code readable for someone still learning software engineering.
- Use foundational, direct, clean names for functions, variables, structs, modules, and files.
- Prefer names that state the component's concrete responsibility over clever, vague, or product-shaped names.
- Add abstractions only when they preserve or clarify the component boundaries.
- Avoid adding features just because they are convenient.
- Do not introduce config systems until the current hardcoded path becomes a real limitation.
- Do not reintroduce slash commands unless explicitly requested.
- Do not add agent/tool behavior until explicitly requested.
- Keep dependencies small and justified.

## Runtime Behavior

The current expected behavior:

```text
start program
read user input
append user message
stream model response
append assistant response
print assistant response
repeat
```

Exit should be Ctrl-C or Ctrl-D.

If a model request fails, let the error stop the program. Do not silently continue after failed inference.

## Verification

After code changes, run:

```bash
cargo fmt
cargo check
cargo test
```

A smoke test may be run with:

```bash
printf 'Hello\n' | cargo run --quiet
```

Windie currently auto-starts the local Bifrost binary from the sibling
`../bifrost` workspace when Bifrost is not already running.
