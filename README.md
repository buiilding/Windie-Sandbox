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
  <a href="https://discord.gg/windie"><img src="https://img.shields.io/badge/Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Discord"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/buiilding/Windie-Sandbox/actions"><img src="https://img.shields.io/badge/Build-Passing-brightgreen?style=for-the-badge" alt="Build: Passing"></a>
  <a href="AGENTS.md"><img src="https://img.shields.io/badge/Agents-AGENTS.md-lightgrey?style=for-the-badge" alt="Agents: AGENTS.md"></a>
</p>

**AI that lives on your computer.**

Windie is a local AI runtime written in Rust. It provides durable conversation trees, explicit control over model context, permissioned tool execution, and extensibility through MCP servers, plugins, and skills. Running as a local daemon, Windie gives the AI a persistent presence on your computer.

```bash
curl -sL https://windieos.com/install | sh
```

On Windows PowerShell:

```powershell
irm https://windieos.com/install.ps1 | iex
```

---

## What Windie Is

Windie is a **layer, not a lock-in**. It's the minimal local harness that other software — agents, tools, workflows — builds on top of. Think of it as the quiet ground floor of an ambient AI operating layer, sitting on your machine, doing exactly what you tell it to and nothing you don't.

Three principles guide everything Windie does:

- **Foundational** — A minimal local harness other software builds upon. Not a platform trying to own your workflow — the ground floor underneath it.
- **Transparent** — You always know what your agent is doing and why. No hidden calls, no black boxes. Every capability is legible, inspectable, and yours to revoke.
- **Yours** — Your harness, your data, your agent, your behavior. It lives on your computer and adapts to you — never the other way around.

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

### MCPs (5)

| Provider | Author | Description |
|---|---|---|
| **Cua Driver** | trycua | Native computer-use driver — click, type, and navigate your desktop like a human would |
| **Blender** | ahujasid | Model, light, and render from a prompt |
| **Desktop Commander** | wonderwhy-er | Filesystem, shell, and process control |
| **Basic Memory** | basicmachines-co | Portable, plain-text, persistent knowledge |
| **Brightdata** | brightdata | Fetch the live web, at scale |

#### Cua Preview

<p align="center">
  <img src="assets/cua-preview.gif" alt="Cua Preview" width="100%">
</p>

#### Blender Preview

<p align="center">
  <img src="assets/blender-preview.gif" alt="Blender Preview" width="100%">
</p>

#### Desktop Commander Preview

<p align="center">
  <img src="assets/DC-preview.png" alt="Desktop Commander Preview" width="100%">
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
