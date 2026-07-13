# Windie

Windie is a minimal local AI runtime foundation for computers.

It is for developers who care about security, performance, and low-level AI
systems that live inside the local operating environment. The goal is not to
build a chat app first. The goal is to build small, inspectable runtime
primitives that can later support chat, shell tools, browser use, computer use,
scheduled wakeups, local memory, and other computer-native AI capabilities
through the same explicit boundaries.

Windie currently provides a Rust CLI, a localhost developer API, SQLite-backed
conversation storage, model-facing context construction, and a small developer
frontend under `dev/` for exercising the runtime primitives.

## Why This Exists

Most AI product surfaces treat the local computer as a thin client. Windie takes
the opposite direction: the local runtime should own local state, local
permissions, local files, local tool execution, and local inspection. Provider
inference is delegated to Bifrost through one OpenAI-compatible path.

The project is intentionally small and direct. Each primitive should be easy to
read, test, replace, and audit.

## What Windie Owns

Windie owns:

- local interaction flow
- conversation trees and active path selection
- model-facing context construction
- SQLite persistence
- localhost developer API primitives
- local image input handling
- explicit tool execution policy
- built-in local tool execution primitives
- terminal and JSON output formatting

Bifrost owns provider routing, provider keys, model calls, and LLM
observability. Clients such as the CLI or developer inspector own user
interface surfaces.

## Current Shape

Important runtime files:

- `src/main.rs` wires components together.
- `src/cli.rs` parses startup arguments into typed commands.
- `src/api.rs` exposes the localhost developer API.
- `src/operation.rs` coordinates shared CLI/API operations.
- `src/conversation.rs` defines message, role, identifier, tool schema, and
  assistant metadata types.
- `src/context.rs` builds model-facing context from the active conversation
  path.
- `src/store.rs` owns SQLite persistence.
- `src/runtime.rs` coordinates one-shot query flows.
- `src/llm.rs` owns the OpenAI-compatible Bifrost HTTP request path.
- `src/policy.rs` owns tool execution policy decisions.
- `src/shell.rs` executes Windie's built-in `run_shell` tool.
- `src/perf.rs` owns local benchmark timing.

Developer-facing references:

- `commands.md` is the concrete CLI command reference.
- `dev/README.md` explains the localhost developer clients.
- `benches/README.md` explains benchmark fixtures and comparison.
- `AGENTS.md` records the project rules, boundaries, and current scope.

## Local Development

Install Windie and prepare its user-local runtime layout:

```bash
curl -fsSL https://github.com/buiilding/Windie-Sandbox/releases/latest/download/install.sh | sh
```

This installs the `windie` binary, creates `~/.windie`, creates
`~/.windie/.env` when missing, prepares Bifrost and benchmark directories, and
verifies `npx` for the public Bifrost runtime.

Set provider keys explicitly:

```bash
windie env OPENAI_API_KEY=<key>
windie env OPENROUTER_API_KEY=<key>
```

Install or verify approved MCP dependencies:

```bash
windie install cua-driver
windie install blender-mcp
```

Build and check the Rust runtime:

```bash
cargo fmt
cargo check
```

Start the localhost developer API:

```bash
target/release/windie api
```

The API listens on `http://127.0.0.1:8787`, starts Bifrost when needed, and
uses a user-local API token from `~/.windie/api-token` unless
`WINDIE_API_TOKEN` is set.

Open the developer inspector:

```bash
target/release/windie inspector
```

The inspector command starts the local React inspector when needed and opens the
browser with the API token already attached.

## Provider Path

Windie talks to Bifrost at:

```text
http://localhost:8080/v1
```

Windie uses an OpenAI-compatible request path. Bifrost handles provider
unification for OpenAI, Anthropic, Ollama, vLLM, and other providers.

The API starts Bifrost on launch. You can still start or stop Bifrost
explicitly through Windie:

```bash
windie gateway start
windie gateway stop
```

Provider secrets should stay outside source control in the explicit provider
key environment used for Bifrost startup.

## Contribution Bar

Windie is foundation code. Contributions should keep the runtime:

- secure by default
- explicit about permission boundaries
- fast enough to benchmark locally
- inspectable through typed data and JSON views
- small enough for one developer to reason about
- strict about module ownership
- boring in the best way

Prefer typed runtime contracts over loose strings and ad hoc JSON. Keep provider
HTTP details in `src/llm.rs`, API route mapping in `src/api.rs`, CLI argument
parsing in `src/cli.rs`, persistence in `src/store.rs`, and model context
construction in `src/context.rs`.

Do not add broad product surfaces before the primitive exists cleanly. The CLI
and localhost API are current test harnesses for the runtime, not the whole
product.

## Good First Areas

Good contributions are small and boundary-respecting:

- add focused tests around an existing primitive
- improve typed contracts around runtime data
- tighten API/CLI parity for an existing operation
- improve provider-free benchmark coverage
- make inspection output clearer without moving ownership
- document command behavior in `commands.md`

Before changing architecture, read `AGENTS.md`. It is the source of truth for
current scope and ownership boundaries.
