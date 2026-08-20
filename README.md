<p align="center">
  <img src="assets/Wordmark.png" alt="Windie" width="100%">
</p>

# Windie
<p align="center">
  <a href="https://github.com/buiilding/Windie-Sandbox">Windie</a> | <a href="https://windieos.com">Website</a>
</p>
<p align="center">
  <a href="https://github.com/buiilding/Windie-Sandbox/releases"><img src="https://img.shields.io/badge/Release-GitHub-blue?style=for-the-badge" alt="Release"></a>
  <a href="https://windieos.com/docs"><img src="https://img.shields.io/badge/Docs-windieos.com-FFD700?style=for-the-badge" alt="Documentation"></a>
  <a href="https://discord.gg/VEu3pgAr5"><img src="https://img.shields.io/badge/Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Discord"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/buiilding/Windie-Sandbox/actions"><img src="https://img.shields.io/badge/Build-Passing-brightgreen?style=for-the-badge" alt="Build: Passing"></a>
  <a href="AGENTS.md"><img src="https://img.shields.io/badge/Agents-AGENTS.md-lightgrey?style=for-the-badge" alt="Agents: AGENTS.md"></a>
</p>

**Control what your AI sees.**

*AI that lives on your computer.*

Windie is a local AI runtime written in Rust that lets you control what your AI sees. Edit, add, or remove messages, branch from any point, and keep every path in one conversation tree. Its daemon keeps sessions running independently of the UI, with permissioned tools and open extensions.

```bash
curl -sL https://windieos.com/install | sh
```

On Windows PowerShell:

```powershell
irm https://windieos.com/install.ps1 | iex
```

## Repository development workflow

The public `windie` binary is the only CLI.

From the repository root, run:

```bash
cargo run --bin windie -- dev run gateway
cargo run --bin windie -- dev run api
cargo run --bin windie -- dev run inspector
cargo run --bin windie -- dev run tray
```

Each command runs exactly one foreground development component. Start the
components you need in separate terminals; Windie intentionally has no
aggregate development runner.

### Tray notification probe (macOS)

After restarting the development API and tray, send the development-only
completion probe from a third terminal:

```bash
curl -i -X POST http://127.0.0.1:8787/api/dev/tray-notifications/assistant-completed
```

The response includes `tray_receivers: 1` when the tray is connected, then the
tray shows a native `Windie — Assistant finished` notification. This probe does
not create a conversation, session, or durable runtime event.

For normal runtime work, the tray listens only for durable final
`session.completed` events. It shows a whitespace-normalized preview of the
actual final assistant text (up to 240 characters), never a tool call or tool
result; the cursor is stored locally so a reconnect does not repeat
notifications already shown.

Installed macOS releases run the tray from `Windie Tray.app`, so clicking a
completed-response notification opens the exact hosted Inspector session at
`https://app.windieos.com/sessions/<session-id>`. A checkout's `cargo run ...
dev run tray` remains intentionally unbundled: it can show the development
notification but cannot receive an operating-system notification click callback.

For any normal CLI command during development, use the same pattern:

```bash
cargo run --bin windie -- status
cargo run --bin windie -- bench
```

For release packaging:

```bash
cargo run --bin windie -- release build
cargo run --bin windie -- release install
cargo run --bin windie -- release verify
source ./scripts/activate_windie
windie status
```

For marketplace publishing, mark a package with `"marketplace": { "publish": true }`
in its `plugin.json`. Its `presentation.repository_url` may optionally name the
canonical `https://github.com/<owner>/<repository>` source project. Then run:

```bash
cargo run --bin windie -- marketplace build
cargo run --bin windie -- marketplace publish
```

The build discovers opted-in packages, creates the local test catalog at
`target/local-marketplace`, and generates its `index.json`. Publishing creates
immutable `.tar.gz` assets in an automatically named GitHub Release, then deploys only the
catalog, manifests, README files, and icons to Vercel. It requires authenticated
`gh` and `vercel` CLIs and does not modify the checked-in `marketplace/index.json`
fixture.

In development, React uses HMR when `windie dev run inspector` starts it. The
Rust API and Bifrost gateway are built by their individual `windie dev run`
commands; rerun the relevant command after backend source changes. The release
Inspector embeds the frontend and is intentionally not hot reloaded.
Installations in separate
worktrees can run together by assigning distinct
`WINDIE_GATEWAY_PORT`, `WINDIE_API_PORT`, and `WINDIE_INSPECTOR_PORT` values.

---

## What Windie Is

Windie is the runtime beneath your AI—not another chat app. It runs locally and gives you direct control over the context your AI receives: edit, add, or remove messages; branch from any point; and keep every path in one conversation tree. Its daemon keeps the work running independently of the Inspector, while agents, tools, and workflows build on top.

Three principles guide everything Windie does:

- **Context is yours** — Edit what the AI sees and branch without losing the original.
- **Execution persists** — Sessions continue independently of the interface.
- **The system stays open** — Bring your own models, tools, providers, and workflows.

---

## Full Control Over Context

<p align="center">
  <img src="assets/Inspector-preview.png" alt="Windie Inspector" width="100%">
</p>

Conversations in Windie aren't flat chat logs — they're **trees**.

Every conversation is made up of **sessions**, and each session is a **branch**: a specific path through the tree that defines exactly what context gets sent to the LLM. Branch off at any point, explore a different direction, and come back — nothing is overwritten, nothing is lost.

And because you can see the whole tree, you can edit it:

- Modify or delete any message — yours, the assistant's, even tool calls and tool outputs
- Rewrite history to steer a conversation without starting over
- Curate exactly what context the model sees, message by message

No black-box context window. You control what the AI knows, every step of the way.

---

## Extensions for the Harness

<p align="center">
  <img src="assets/extension-lib-preview.png" alt="Windie Extensions Library" width="100%">
</p>

Windie's capabilities come from a growing **registry** of MCPs, plugins, and skills.

Windie doesn't ship with a fixed toolbox — it can **give itself tools based on the context of your task** in order to get the job done.

Two built-in tools drive this:

| Tool | Purpose |
|---|---|
| `list_providers` | Discover which tool providers are available |
| `attach_provider` | Attach a provider on demand, mid-conversation |

When a task needs a capability Windie doesn't currently have attached, it looks, finds it, and attaches it — live, in front of you.

### MCPs (7)

| Provider | Author | Description |
|---|---|---|
| **Cua Driver** | trycua | Native computer-use driver — click, type, and navigate your desktop like a human would |
| **Blender** | ahujasid | Model, light, and render from a prompt |
| **Desktop Commander** | wonderwhy-er | Filesystem, shell, and process control |
| **Basic Memory** | basicmachines-co | Portable, plain-text, persistent knowledge |
| **Brightdata** | brightdata | Fetch the live web, at scale |
| **Chrome DevTools** | Chrome DevTools team | Inspect, debug, and automate a separate persistent Chrome session |
| **Parallel Search** | Parallel | Search the live web and extract content from URLs through a hosted MCP |

#### Cua Preview

<p align="center">
  <img src="assets/cua-preview.gif" alt="Cua Preview" width="100%">
</p>

#### Blender Preview

<p align="center">
  <img src="assets/blender-preview.gif" alt="Blender Preview" width="100%">
</p>

#### Desktop Commander Preview

Desktop Commander capability exposed through Windie; this is not Windie's Inspector.

<p align="center">
  <img src="assets/DC-preview.png" alt="Desktop Commander capability exposed through Windie" width="100%">
</p>

### Plugins (0)
*Coming soon.*

### Skills (0)
*Coming soon.*

The registry is open — anyone can build and publish new MCPs, plugins, and skills for the harness.

---

## Model Providers

Windie is model-agnostic. Bring your own key, run locally, or use whatever provider fits your workflow. Currently supported:

Anthropic · Azure · Bedrock · Bedrock Mantle · Cerebras · Cohere · Deepseek · Elevenlabs · Fireworks · Gemini · Groq · Huggingface · Mistral · Nebius · Ollama · OpenAI · Opencode Go · Opencode Zen · OpenRouter · Parasail · Perplexity · Replicate · Runware · Runway · Sarvam · SGL · Vertex · vLLM · Wafer · xAI

Configure any provider with a simple API key — or run fully local with Ollama, SGLang, or vLLM.

> **Recommended setup:** Kimi K2, via a Kimi Code subscription (not the raw Moonshot API). Kimi Code is subscription-based rather than usage-metered, so you get significantly more usage for the price — and Kimi K2 holds up well against much more expensive frontier models at a fraction of the cost.

---

## Why Windie

- **No cloud lock-in** — swap models and providers freely
- **No black boxes** — inspect every tool call, every context change, every decision
- **No fixed toolbox** — Windie extends itself as your tasks demand
- **No bloat** — one install script, one quiet harness

---

## Get Started

```bash
curl -sL https://windieos.com/install | sh
```

- [Documentation](#)
- [Registry](#)
- [GitHub](#)

---

*Put AI where your computer is.*
