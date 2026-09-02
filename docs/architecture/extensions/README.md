# Extensions

Extensions let Windie add capabilities without hard-coding every integration
into the runtime.

## How the parts fit together

- **Plugin** — the installable package boundary. A plugin can contain one or
  more MCPs, skills, app connectors, or any combination of them.
- **MCP** — the protocol and provider boundary for executable tools.
- **Skill** — reusable text instructions that an LLM can follow.
- **App connector** — a component for integrating an external application.
- **Marketplace** — the catalog and distribution source for published plugins.

## Main flow

1. Windie discovers a plugin from the marketplace or a bundled package.
2. Windie validates and installs the plugin and its declared components.
3. The runtime indexes the installed capabilities. The model can read a skill
   when it needs instructions or attach an MCP when it needs its tools.
4. Approved tool calls run through their provider, and the runtime persists the
   results within the conversation.

Every extension remains subject to Windie's component state, permission
boundaries, and tool-approval policy.

## Detailed pages

- [Plugins](plugins.md)
- [MCP](mcp.md)
- [Skills](skills.md)
- [App connectors](app-connectors.md)
- [Marketplace](marketplace.md)
