# Skills

## Purpose

Skills are instructions: reusable, copyable text that an agent can follow to
complete a task or workflow. They are text-native and usually written in
Markdown, so an LLM can read the instructions and chain the tools it already
has to produce an output.

A skill should contain the knowledge, steps, inputs, outputs, and guardrails
needed for its workflow. It is reusable across tasks, but it is not independent
of all context: Windie loads it alongside the current conversation and the
tools available to that conversation. A skill cannot create a tool that Windie
does not already provide.

Skills are often less expensive than MCPs when the workflow can be expressed
as instructions. An MCP exposes callable tools with schemas and parameters,
which can reduce the number of steps the model has to plan but adds schema
tokens and may require a running provider process or network connection.

## Owns

Skills are packaged as `skill` components inside a plugin. The component points
to a Markdown file, normally `SKILL.md`. Windie reads that file as package
content and extracts its name and description for the plugin catalog. The
complete instructions are not put in the catalog up front.

The common Agent Skills format uses a skill directory with a required
`SKILL.md`. That file normally has YAML frontmatter containing `name` and
`description`, followed by Markdown instructions. The broader format also
allows optional `scripts/`, `references/`, and `assets/` directories. Windie's
current skill loader reads and validates the referenced Markdown document; it
does not execute skill scripts or automatically load those optional resources.

## Does not own

Skills do not execute tools, start processes, or provide an MCP connection. The
runtime and tool providers own execution, while Windie's approval policy still
controls any tool call the skill asks the model to make.

Skill instructions are not user-authored messages. When the model requests one,
Windie returns the complete text through the built-in `windie__read_skill`
tool. The runtime may persist that response as a normal `role: tool` result,
but the installed package remains the source of truth for the skill.

## Main flow

1. An installed plugin declares a `skill` component in `plugin.json` and points
   it to a Markdown skill document.
2. Windie validates the document, reads its name and description, and includes
   that compact metadata in the generated plugin index. This is the discovery
   step and has a small token cost.
3. When the model decides the skill is relevant, it calls
   `windie__read_skill` with the exact plugin and skill IDs from the index.
4. Windie loads the complete package-owned instructions and returns them as the
   built-in tool result. The model can then follow the workflow and call any
   already available tools in later steps.

## Important invariants

- Skill metadata is disclosed before activation, while the full instruction
  text is loaded only when the model requests that skill. This progressive
  disclosure keeps the initial context smaller.
- A skill is text and instructions, not an executable provider. Reading one
  does not install, attach, enable, or approve an external tool.
- The package-owned skill document remains authoritative. Windie does not
  silently replace it with conversation text or with a model-generated copy.
- Skill instructions can guide the model to chain tools, but those tools must
  already be available through Windie's runtime and remain subject to approval.
- The common format recommends keeping the main `SKILL.md` focused and moving
  detailed material into referenced resources. Windie's current implementation
  does not yet provide a general resource-loading or script-execution layer for
  those optional directories.

## Related code

- [`src/plugin/manifest.rs`](../../../src/plugin/manifest.rs) — parses the
  skill document and its `name` and `description` metadata.
- [`src/plugin/store.rs`](../../../src/plugin/store.rs) — loads complete skill
  instructions from installed plugin packages.
- [`src/plugin/catalog.rs`](../../../src/plugin/catalog.rs) — projects skill
  metadata into the generated plugin index.
- [`src/tool/builtin.rs`](../../../src/tool/builtin.rs) — defines
  `windie__read_skill`.
- [`src/runtime/tool_execution.rs`](../../../src/runtime/tool_execution.rs) —
  executes the built-in skill read and returns its instructions.
- [Agent Skills specification](https://agentskills.io/specification) — common
  `SKILL.md` format, optional resources, and progressive disclosure.
- [OpenAI: Using skills](https://openai.com/academy/skills/) — reusable,
  shareable workflow guidance and `SKILL.md` usage.
- [Anthropic: Agent Skills](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills) —
  the origin of the open format and its progressive-disclosure model.
