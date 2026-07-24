# Windie

**Build AI that can use a computer.**

Windie is a local runtime for AI agents. It gives an assistant durable state,
exact context, explicit tools, and a way to pause for approval and resume
without losing the thread.

Windie is not a personal assistant or messaging gateway. It is the small
runtime underneath one.

- [Website](https://windieos.com)
- [GitHub](https://github.com/buiilding/Windie-Sandbox)

<!-- Add the inspector screenshot at docs/inspector-preview.png. -->
![Windie inspector preview](docs/inspector-preview.png)

## What Windie gives an agent

An agent running on Windie can:

- branch a conversation from any message
- inspect the exact context sent to the model
- expose only the tools attached to the current conversation
- wait for approval before using a sensitive capability
- queue new work while a session is running
- resume after an approval, interruption, or client disconnect
- show the assistant response, reasoning, tool calls, and results as they happen

The result is an agent whose state and actions remain visible to the person
running it.

## One run, end to end

Suppose an agent needs to inspect a Blender scene. Windie stores the request in
a conversation tree, builds the model context for the selected branch, exposes
the attached Blender tool, and sends the request to the model. When the model
asks to call Blender, Windie applies its approval policy, pauses if approval is
needed, stores the result, and continues the run from the new message head.

```text
input
  → session
  → conversation head
  → model context
  → assistant response or tool call
  → approval policy
  → extension execution
  → stored result
  → next model turn
```

## Conversations are trees

Windie stores messages as a durable tree instead of a flat transcript. Every
run starts from an explicit message head, and the model sees the path from the
root to that head.

That makes branching a first-class runtime operation:

- fork at any message
- insert a new child under a selected head
- inspect every branch
- truncate descendants
- remove or replace messages
- run separate sessions from different heads

The complete tree stays in SQLite. The model receives only the context resolved
for the selected head, including the system prompt, attached tools, and any
applicable compaction checkpoint.

## Extensions

Extensions are how agents gain capabilities.

Windie treats an extension as a provider-backed capability, not just a prompt
file or a name in a catalog. A provider can declare its tools, launch command,
dependencies, secrets, platforms, permissions, scope, authentication, setup
instructions, and health state.

The capability boundary is explicit:

```text
provider installed
  → provider enabled
  → tools discovered
  → tool attached to a conversation
  → schema exposed to the model
  → tool call checked by policy
  → execution approved or denied
```

Provider availability does not grant the model access to every tool. Windie also
provides two built-in control tools for discovering and attaching providers:

- `windie__list_providers`
- `windie__attach_provider`

The current extension catalog is MCP-based. Plugin and skill provider families
are future extension surfaces.

### Current providers

| Provider | Capability | Scope |
| --- | --- | --- |
| [CUA Driver](https://github.com/trycua/cua) | Computer control | Local |
| [Desktop Commander](https://github.com/wonderwhy-er/DesktopCommanderMCP) | Filesystem and process control | Local |
| [Blender MCP](https://github.com/ahujasid/blender-mcp) | Blender and 3D workflows | Local |
| [Basic Memory](https://github.com/basicmachines-co/basic-memory) | Local memory and notes | Local |
| [Bright Data](https://github.com/brightdata/brightdata-mcp) | Live web and data access | Cloud |

## Local by design

Windie keeps the runtime, conversation state, sessions, provider state, and
approval decisions on the user's machine.

Model inference can be local or remote. Windie talks to [Bifrost](https://github.com/maximhq/bifrost)
through one OpenAI-compatible Responses path at:

```text
http://localhost:8080/v1
```

Bifrost handles routing for OpenAI, Anthropic, Ollama, vLLM, and other supported
providers. The Windie runtime does not need a separate code path for each model
provider.

## Run Windie

Install the latest release:

```bash
curl -fsSL https://github.com/buiilding/Windie-Sandbox/releases/latest/download/install.sh | sh
```

Configure model providers and approved extensions:

```bash
windie onboard
```

Start the localhost API:

```bash
windie api
```

Open the developer inspector:

```bash
windie inspector
```

Windie keeps its local runtime data under `~/.windie`. The API listens on
`http://127.0.0.1:8787` and uses a per-process API token.

## Developer surfaces

Windie has four local clients:

- **Rust CLI** for conversations, trees, sessions, approvals, tools, providers,
  gateway control, and benchmarks.
- **Localhost JSON API** for applications and test harnesses.
- **React inspector** for conversation trees, model context, sessions,
  approvals, providers, and extension state.
- **Plain browser client** under `dev/windie-ui` for a smaller chat and tool
  surface.

The clients call runtime operations. They do not own persistence, context
construction, provider execution, or approval policy.

## Current scope

Windie is early foundation code. The current focus is reliable local runtime
primitives and a localhost developer harness.

Implemented today:

- durable conversation trees and explicit execution heads
- session supervision, queued inputs, approvals, and event streaming
- SQLite persistence for messages, images, sessions, tools, and provider state
- provider-backed MCP tools with lifecycle and health management
- computer, filesystem, Blender, memory, and web-data integrations
- model-context inspection and compaction checkpoints
- typed CLI, API, runtime, storage, and provider boundaries

Not currently part of the runtime:

- messaging-channel integrations
- scheduled or file-event wakeups
- a dynamic public extension marketplace
- plugin and skill packages
- self-improving agent behavior
- remote worker orchestration

## Development

From the repository root:

```bash
cargo fmt
cargo check
cargo test
```

Read [commands.md](commands.md) for the concrete CLI reference, [dev/README.md](dev/README.md)
for the local developer clients, and [AGENTS.md](AGENTS.md) for ownership
boundaries and project rules.

The runtime is organized by responsibility:

- `src/conversation/` — messages, roles, IDs, parts, and assistant metadata
- `src/context.rs` — model-facing context construction
- `src/runtime/` — model turns and tool execution flow
- `src/session/` — durable session state and live supervision
- `src/tool/` — tool schemas, providers, results, and approval data
- `src/tool_provider/` — provider discovery, lifecycle, and execution
- `src/store/` — SQLite persistence
- `src/api/` — localhost routes, authentication, JSON, and SSE
- `src/llm/` — Bifrost HTTP and provider wire serialization

## Contributing

Windie is foundation code. Prefer one clear primitive at a time.

Keep runtime actions explicit, typed, inspectable, and replaceable. Add tests
before expanding the product surface, and keep provider-specific behavior at
the provider boundary.
