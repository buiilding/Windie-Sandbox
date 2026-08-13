# Windie plugins

Windie supports two plugin sources behind one runtime-facing contract:

1. **Code-owned plugins** are reviewed and compiled into the Windie binary.
   They are the trusted baseline for capabilities that need native Windie
   integration or a tightly controlled local runtime.
2. **Package plugins** are local, file-based bundles using the Codex-shaped
   package layout. They are discovered and validated at runtime, so adding a
   package does not require changing Rust code or rebuilding Windie.

Both sources expose the same composition model:

- a plugin manifest gives the model a compact purpose and ownership summary;
- skills provide instructions that Windie reads on demand;
- MCP declarations describe tools that are registered and discovered only
  when the model attaches the plugin.

## Package layout

The canonical package shape is:

```text
my-plugin/
├── .codex-plugin/
│   └── plugin.json
├── skills/
│   └── workflow/
│       ├── SKILL.md
│       ├── MACOS.md
│       └── references/
│           └── example.md
├── .mcp.json                 # optional
├── bin/                      # optional MCP launcher or package assets
├── assets/                   # optional icons and other package assets
└── README.md                 # optional human documentation
```

`plugin.json` uses the Codex-compatible fields needed by Windie:

```json
{
  "name": "computer-use",
  "version": "1.0.0",
  "description": "Control local computer applications.",
  "author": { "name": "Windie" },
  "skills": "./skills/",
  "mcpServers": "./.mcp.json",
  "interface": { "displayName": "Computer Use" }
}
```

Each skill directory must contain `SKILL.md`. Other files in that directory
are bounded supporting documents. `read_skill` returns the entrypoint by
default and includes the names of supporting files so the model can request
one explicitly. Paths cannot be absolute, escape the skill directory, or use
symlinks.

`.mcp.json` follows the standard MCP server map:

```json
{
  "mcpServers": {
    "computer-use": {
      "command": "./bin/computer-use-client-launcher",
      "args": ["mcp"],
      "cwd": ".",
      "env_vars": ["CODEX_HOME"]
    }
  }
}
```

Loading a package never starts its command. Windie validates the manifest,
indexes skills, rejects unsafe paths and symlinks, and computes a content hash.
MCP processes start only after `attach_plugin` and the existing provider setup,
permission, discovery, and catalog flow succeeds.

## Runtime flow

The initial model context contains only:

- `read_skill(plugin_id, skill_id, path?)`;
- `attach_plugin(plugin_id)`;
- a compact `Available plugins` system message.

The system message is rebuilt for each model-context build, so it reflects the
current discovered package set and MCP setup state. Full skill content and MCP
tool schemas are intentionally absent until requested.

`read_skill` loads one file from either the compiled code-owned bundle or the
package directory. `attach_plugin` registers package MCP declarations with the
existing MCP registry, installs/discovers the provider catalog through the
existing Store lifecycle, and attaches the persisted tool schemas to the
conversation. The newly attached MCP tools are available on the next model
turn.

## Installation and discovery

Local packages are installed under:

```text
~/.windie/plugins/<plugin-id>/<version>/
```

The package installer validates the source before copying it and refuses to
silently replace a different package with the same ID and version. The default
plugin registry discovers both direct package roots and versioned package
directories under this store.

Local marketplace JSON can index local package directories and Git metadata.
Only local entries are currently resolvable by Windie's filesystem installer;
network fetching is intentionally not implicit. A future remote installer can
fetch into the same versioned store and reuse the exact same package validator.

## Ownership boundaries

```text
plugins/       composition, discovery, package validation, attach orchestration
skills/        bounded instruction files and skill reads
mcp/           MCP transport, provider lifecycle, discovery, and execution
builtin_tools/ model-facing Windie control tools
tool/          shared tool/provider/schema contracts
```

This keeps the user-facing extension unit as the plugin while preserving
replaceable lower-level implementations. Code-owned plugins and package
plugins can therefore coexist without duplicating MCP transport or tool
attachment logic.
