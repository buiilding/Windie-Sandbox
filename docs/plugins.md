# Windie plugins

Windie supports two plugin origins behind one runtime-facing contract:

1. **Curated plugin definitions** are reviewed and compiled into the Windie
   binary. They are trusted installation recipes for capabilities that need
   native Windie integration or a tightly controlled local runtime.
2. **Package plugins** are local, file-based bundles using the Codex-shaped
   package layout. They are discovered and validated at runtime, so adding a
   package does not require changing Rust code or rebuilding Windie.

After installation, a curated plugin is materialized into the same versioned
package store as a package plugin. The curated Rust definition remains the
source of trust, permissions, and installation policy; the installed package
owns the file-backed manifest, skills, provenance, and package MCP declaration.

Both origins expose the same composition model. A package may contain either
primitive independently or both together:

- a plugin manifest gives the model a compact purpose and ownership summary;
- skills provide instructions that Windie reads on demand;
- MCP declarations describe tools that are registered and discovered only
  when the model attaches the plugin.

The runtime classifies the package by its contents:

```text
skills + MCP declarations = plugin
skills only              = standalone skill package
MCP declarations only    = standalone MCP package
```

## Package layout

The canonical package shape is:

```text
my-plugin/
├── .codex-plugin/
│   └── plugin.json
├── skills/                   # optional when .mcp.json is present
│   └── workflow/
│       ├── SKILL.md
│       ├── MACOS.md
│       └── references/
│           └── example.md
├── .mcp.json                 # optional when skills/ is present
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

At least one of `skills` or `.mcp.json` is required. Loading a package never
starts its command. Windie validates the manifest, indexes skills when present,
rejects unsafe paths and symlinks, and computes a content hash.
MCP processes start only after `attach_extension` and the existing provider setup,
permission, discovery, and catalog flow succeeds.

## Runtime flow

The initial model context contains only:

- `read_skill(plugin_id, skill_id, path?)`;
- `attach_extension(target)` where target is `plugin:<id>` or `mcp:<id>`;
- a compact catalog of plugins, standalone skills, and standalone MCP servers.

The system message is rebuilt for each model-context build, so it reflects the
current discovered package set and MCP setup state. Full skill content and MCP
tool schemas are intentionally absent until requested.

The Inspector exposes these generated system messages in a read-only **model
system context** preview. The editable conversation system prompt remains
separate, so runtime-generated catalog text is visible without being persisted
as user-authored conversation state.

`read_skill` loads one file from the installed package directory. Before a
curated plugin is installed, its skill is listed but cannot be read. Once
installed, `attach_extension` registers the package MCP declaration with the
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
directories under this store. A materialized curated package replaces its
uninstalled code-owned catalog entry at runtime; it is not treated as a
duplicate.

For example, installing the curated CUA Driver plugin performs both upstream
installations and then materializes:

```text
~/.windie/plugins/cua-driver/<cua-driver-version>/
├── .codex-plugin/plugin.json
├── .mcp.json
├── windie-provenance.json
└── skills/cua-driver/
    ├── SKILL.md
    ├── MACOS.md
    ├── WINDOWS.md
    └── LINUX.md
```

The CUA executable is installed by the approved MCP installer. Windie then
runs `cua-driver skills install --all-platforms`, validates the directory
returned by `cua-driver skills path`, and copies the upstream skill pack into
the versioned package. Installation and MCP attachment remain separate model
runtime actions.

Local marketplace JSON can index local package directories and Git metadata.
Only local entries are currently resolvable by Windie's filesystem installer;
network fetching is intentionally not implicit. A future remote installer can
fetch into the same versioned store and reuse the exact same package validator.

## Current limitations

The compact extension catalog currently exposes identity, purpose, skills, MCP
server references, ownership where available, and lifecycle status. It does not
yet print permissions, dependencies, or platform requirements in the model
context, even though some of that metadata is already available in MCP
manifests. Those fields should be added to the catalog before installation or
attachment when the runtime's extension metadata contract is expanded.

## Ownership boundaries

```text
plugins/       composition, discovery, package validation, attach orchestration
skills/        bounded instruction files and skill reads
mcp/           MCP transport, provider lifecycle, discovery, and execution
builtin_tools/ model-facing Windie control tools
tool/          shared tool/provider/schema contracts
```

This keeps the user-facing extension unit as the plugin while preserving
replaceable lower-level implementations. Curated and marketplace plugins can
therefore coexist without duplicating MCP transport or tool attachment logic.
