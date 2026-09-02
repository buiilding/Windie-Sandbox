# Architecture references

This folder explains Windie by runtime responsibility rather than by Rust
source directory. Start with [the architecture overview](overview.md), then
open the group that owns the concept you want to understand.

## Architecture groups

- [Storage](storage/) — conversation trees, sessions, SQLite,
  execution claims, and durable events.
- [Execution](execution/) — wakeups, context compilation, runtime turns, tool
  execution, and approval.
- [LLM](llm/) — Bifrost, model requests, discovery, and
  provider configuration.
- [Extensions](extensions/) — plugins, MCP components, skills, app connectors,
  and the marketplace.
- [Interfaces](interfaces/) — the HTTP API, SSE, CLI, and Inspector.
- [Components](components/) — the API, gateway, tray, and notifier
  as independently managed processes.

Each group's `README.md` is its overview. The other files in that folder cover
one concept or a narrower implementation detail. Step-by-step workflows belong
in [`../guides/`](../guides/), while the reasoning behind accepted choices
belongs in [`../decisions/`](../decisions/).
