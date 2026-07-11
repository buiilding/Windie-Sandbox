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
frontend under `dev/` for exercising the runtime primitives. Release packages
also include a compiled operator UI served by the localhost API.

## Install

Install the latest published binary and bundled operator UI:

```bash
curl -fsSL https://raw.githubusercontent.com/buiilding/Windie-Sandbox/main/install.sh | bash
windie doctor
windie api
```

`windie api` prints the authenticated operator UI URL. A normal installation
contains Windie only. Bifrost and approved MCP servers use their public,
version-pinned launch interfaces; their source repositories are not bundled.

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
- `src/cli.rs` parses startup arguments into typed commands;
  `src/cli/execute.rs` owns the CLI adapter.
- `src/api.rs` composes the localhost developer API; `src/api/` owns route
  families and their private HTTP types.
- `src/operation.rs` exposes shared contracts; `src/operation/` separates
  conversation, model, tool, and execution operations.
- `src/conversation.rs` defines message, role, identifier, tool schema, and
  assistant metadata types.
- `src/context.rs` builds model-facing context from the active conversation
  path.
- `src/store.rs` owns shared SQLite transaction/tree integrity; `src/store/`
  separates schema, run, conversation, message, tool, image, and compaction
  persistence.
- `src/runtime.rs` coordinates one-shot query flows.
- `src/run.rs` owns durable backend runs and reconnectable event delivery.
- `src/paths.rs` owns installed, development, and override filesystem paths.
- `src/doctor.rs` inspects optional integration prerequisites.
- `src/llm.rs` exposes typed LLM contracts; `src/llm/` separates Bifrost HTTP
  calls, model metadata, Responses request serialization, and SSE decoding.
- `src/policy.rs` owns tool execution policy decisions.
- `src/perf.rs` exposes benchmark options; `src/perf/metrics.rs` owns reports
  and statistics while `src/perf/scenarios/` separates conversation, runtime,
  fixture, and live-provider behavior.

Developer-facing references:

- `docs/architecture/cli.md` is the concrete CLI command reference.
- `dev/README.md` explains the localhost developer clients.
- `benches/README.md` explains benchmark fixtures and comparison.
- `AGENTS.md` records the project rules, boundaries, and current scope.

## Local Development

Install frontend dependencies once:

```bash
scripts/setup.sh
```

Start the isolated Rust API and hot-reloading inspector together:

```bash
scripts/dev.sh
```

The API listens on `http://127.0.0.1:8787` and prints a per-process API token.
The inspector runs at the URL printed by the script:

```text
http://localhost:3000?windie_token=<printed token>
```

Development state defaults to `target/windie-dev-data`, not the installed
runtime database. Frontend changes hot reload. Restart `scripts/dev.sh` after
Rust changes.

The installed operator UI and editable preview are separate surfaces. The
operator UI is bundled beside the installed binary and remains unchanged while
the preview hot reloads. Runtime loops are backend-owned runs, so either UI can
disconnect and replay events without cancelling model or tool work.

Run the full local correctness check before committing:

```bash
scripts/check.sh
```

This runs Rust formatting, tests, clippy, and a production frontend build. It
does not call Bifrost, a model provider, or performance benchmarks.

Promote a tested checkout into a versioned local release:

```bash
scripts/install.sh
```

The install script builds the release binary and operator UI, installs both
under `~/.local/lib/windie/releases/<version>-<revision>/`, and atomically
switches `~/.local/bin/windie`. It assumes checks have already passed and does
not rerun them. A running process continues to use its old release until
explicitly restarted.

## Provider Path

Windie talks to Bifrost at:

```text
http://localhost:8080/v1
```

Windie uses an OpenAI-compatible request path. Bifrost handles provider
unification for OpenAI, Anthropic, Ollama, vLLM, and other providers.

Start or stop Bifrost explicitly through Windie:

```bash
windie gateway start
windie gateway stop
```

Provider secrets should stay outside source control in Windie's canonical
provider environment. Windie uses it for approved MCP providers and passes it
to Bifrost. The default file is `~/.config/windie/providers.env`;
`WINDIE_ENV_FILE` can select another file.

Windie starts unmodified Bifrost `1.6.3` through its public npm package, or the
matching Docker image when npm is unavailable. Environment variables make
configured `env.KEY` references available, but Windie does not create provider
rows. Configure providers through Bifrost at `http://localhost:8080`.

`windie models` returns the configured models reported by public Bifrost. It
does not apply a private chat-completion allowlist; the selected provider owns
capability validation for Windie's Responses request.

Developers testing an official local Bifrost build can set
`WINDIE_BIFROST_BIN=/absolute/path/to/bifrost-http`. Windie never discovers a
sibling checkout implicitly.

## External MCPs

Windie keeps a code-approved provider list but does not package MCP source:

- CUA runs through an explicitly installed `cua-driver mcp`.
- Desktop Commander runs through pinned npm package `0.2.44`.
- Blender MCP runs through pinned Python package `1.6.0`; its Blender addon is
  installed separately.

Run `windie doctor` for prerequisite status and official setup commands. Tools
remain unavailable to a model until attached to a conversation through
Windie's permission boundary.

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
HTTP details in `src/llm/`, API route mapping in `src/api/`, CLI argument
parsing in `src/cli.rs`, persistence in `src/store/`, and model context
construction in `src/context.rs`.

Do not add broad product surfaces before the primitive exists cleanly. The CLI
and localhost API are current test harnesses for the runtime, not the whole
product.

## Commit Workflow

Use normal Git after the local correctness check:

```bash
scripts/check.sh
git commit
git push
```

Performance-sensitive changes can run explicit provider-free benchmarks with
`scripts/bench.sh`. Benchmarks are not required for ordinary commits and their
machine-local output is not appended to commit messages.

## Good First Areas

Good contributions are small and boundary-respecting:

- add focused tests around an existing primitive
- improve typed contracts around runtime data
- tighten API/CLI parity for an existing operation
- improve provider-free benchmark coverage
- make inspection output clearer without moving ownership
- document command behavior in `docs/architecture/cli.md`

Before changing architecture, read `AGENTS.md`. It is the source of truth for
current scope and ownership boundaries.
