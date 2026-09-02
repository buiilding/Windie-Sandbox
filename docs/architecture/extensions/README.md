# Extensions

Extensions let Windie add capabilities without hard-coding every integration
into the runtime. They are distributed as plugins, and each plugin can contain
one or more capability components.

```text
marketplace or bundled package
              │
              v
           plugin
       ┌──────┼──────┐
       v      v      v
      MCP   skill   app connector
       │      │      │
       v      v      v
 executable  text   metadata
   tools   instructions  (current implementation)
```

## Purpose

Windie is the runtime, while extensions provide capabilities that can be
installed and used by that runtime. This keeps the core runtime focused and
gives developers a package boundary for adding MCP tools, reusable skill
instructions, and future app connectors.

## Extension roles

- **Plugin** — the installable package boundary. A plugin can contain one or
  more MCPs, skills, app connectors, or any combination of them.
- **MCP** — the protocol and provider boundary for executable tools.
- **Skill** — reusable text instructions that an LLM can follow.
- **App connector** — a component for describing an external application. The
  current implementation validates and indexes its metadata but does not yet
  connect to the application.
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

## Extension boundaries

- Plugins package and describe components; they do not execute tools or decide
  whether a tool call is allowed.
- MCP providers own executable tool transport and discovery. The runtime still
  controls attachment, approval, and execution boundaries.
- Skills provide text instructions and do not create tools or execute scripts.
- App connectors currently provide metadata only; they do not establish an
  external connection or provide executable schemas.
- The marketplace distributes plugin releases but does not run installed
  components or own their local state.

## References

- [Plugins](plugins.md)
- [MCP](mcp.md)
- [Skills](skills.md)
- [App connectors](app-connectors.md)
- [Marketplace](marketplace.md)
