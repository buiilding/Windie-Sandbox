# Migrating code-owned MCPs to plugins

This guide describes the current Windie migration from code-owned MCP
providers to packaged plugins.

The goal is not to change how Windie uses an MCP. The goal is to move the
provider's identity and connection description out of Rust-owned definitions
and let Windie own the package lifecycle:

```text
marketplace index
  -> installed plugin package
  -> plugin.json
  -> standard MCP server.json plus Windie policy
  -> generic MCP provider
  -> ToolProviderRegistry
  -> dynamic tool discovery and execution
```

Windie still owns the MCP runtime, permission boundary, lifecycle state,
tool-schema persistence, and conversation attachment rules. A plugin does not
automatically attach tools to a conversation.

## Current state

Parallel Search, Desktop Commander, Basic Memory, and Blender MCP now use the
package-owned path. The first packaged local MCPB was a harmless fixture used
to prove the generic mechanism before migrating real providers.
Their package layouts are:

```text
  packages/parallel-search/
  plugin.json
  mcp/server.json
  README.md
  assets/icon.svg

  packages/desktop-commander/
  plugin.json
  mcp/server.json
  mcp/desktop-commander.mcpb
  README.md
  assets/icon.svg

  packages/basic-memory/
  plugin.json
  mcp/server.json
  mcp/basic-memory.mcpb
  README.md
  assets/icon.svg

  packages/blender-mcp/
  plugin.json
  mcp/server.json
  mcp/blender-mcp.mcpb
  README.md
  assets/icon.svg

  packages/chrome-devtools/
  plugin.json
  mcp/server.json
  mcp/chrome-devtools.mcpb
  README.md
  assets/icon.svg

  packages/cua-driver/
  plugin.json
  mcp/server.json
  mcp/manifest.json
  mcp/cua-driver.mcpb
  README.md
  assets/icon.svg
```

Its marketplace entry is:

```text
marketplace/index.json
```

The package foundation currently lives in:

```text
src/plugin/manifest.rs  # typed plugin and MCP component contracts
src/plugin/catalog.rs   # marketplace index contract
src/plugin/store.rs     # validated, versioned local package store
src/plugin/mod.rs       # public package boundary and tests
src/api/plugin.rs       # marketplace and package lifecycle routes
```

The runtime ownership boundaries are:

```text
src/plugin/          package installation, storage, and marketplace metadata
src/mcp/             MCP protocol, transports, MCPB, loading, and tool adapter
src/tool/            Windie-facing tool schemas, registry, policy, and state
src/managed_runtime/ Node and uv runtimes used by packaged local MCPs
src/local/           Windie-local files, processes, tray, and secrets
src/runtime/         model turns, context construction, and wakeups
```

The old `src/tool_provider/` directory is intentionally gone. Provider
identity and tool execution remain useful concepts, but their code now lives
under the tool and MCP domains rather than forming a separate package-shaped
architecture.

The current local store is versioned by plugin ID and release:

```text
~/.windie/plugins/<plugin-id>/<version>/
```

The production installer reads a configured marketplace index, downloads the
selected artifact, verifies its SHA-256 digest, validates the package identity,
copies it into the local store, and registers its MCP components. The bundled
installer remains as a deterministic development fixture.

The production index URL defaults to
`https://marketplace.windieos.com/index.json` and can be overridden for
staging or tests with `WINDIE_MARKETPLACE_INDEX_URL`. Marketplace requests
require HTTPS; HTTP is accepted only for localhost fixtures.

To build and host the current Parallel package locally:

```text
cargo run --bin windie -- marketplace build
cargo run --bin windie -- marketplace serve
```

The development server generates `target/local-marketplace`, calculates the
artifact digest, writes a fresh `index.json`, and serves it at:

```text
http://127.0.0.1:8788/index.json
```

Run Windie with this environment value to exercise the upstream installer
against the local marketplace:

```text
WINDIE_MARKETPLACE_INDEX_URL=http://127.0.0.1:8788/index.json
```

The package loader supports remote Streamable HTTP MCP components and local
MCPB stdio components. Local MCPB runtimes are extracted into the installed
plugin version, launched without a shell, and can receive Windie-declared
isolated HOME, environment values, and JSON configuration files. Package-owned
uv MCPBs are prepared before MCP initialization so dependency installation does
not consume the protocol startup timeout. Desktop Commander uses this path
with a pinned Node runtime and bundled dependencies. Basic Memory uses it with
a pinned Python dependency graph managed by uv while keeping user notes in
Windie’s persistent data directory.

## Interoperability decision

The current `mcp/<component-id>.json` format is Windie’s temporary component
schema. Do not expand it into another proprietary universal MCP format.
Windie should consume and produce the existing MCP ecosystem formats:

```text
server.json
  MCP Registry discovery metadata for remote and installable servers

MCPB manifest.json
  runtime manifest inside a local .mcpb MCP bundle

SKILL.md
  Agent Skills instructions for skill components

plugin.json
  Windie’s outer package manifest for grouping MCPs, skills, and app connectors
```

The MCP Registry defines `server.json` for remote `remotes` and local
`packages`, including package identifiers, transports, versions, and artifact
hashes. See:
`https://modelcontextprotocol.io/registry/about` and
`https://modelcontextprotocol.io/registry/package-types`.

The MCPB specification defines `manifest.json` for a local MCP bundle. A
future local MCP package such as CUA Driver should use an MCPB artifact rather
than a Windie-only launch schema. See:
`https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md`.

The Windie marketplace index may remain Windie-specific because it catalogs
multi-component plugins. For each MCP release, however, it should reference
or be generated from standard `server.json` metadata. Windie-specific fields
such as conversation permissions, provider lifecycle state, and cleanup
ownership should be namespaced extensions or outer `plugin.json` metadata.

Claude Code and Codex have different outer plugin/configuration formats, so
Windie should provide adapters or exporters for them rather than pretending
that Windie’s complete `plugin.json` is a universal harness plugin. The
interoperable units are MCP protocol/server metadata, MCPB packages, and
`SKILL.md` skills.

## Future skill and app components

Do not create empty runtime folders in Windie's Rust source tree for skills or
app connectors before those runtimes exist. Windie should add a source module
only when a real component requires executable loading or lifecycle behavior.

For now, an MCP-only plugin needs no `src/skill/` directory. When Windie
implements its first skill runtime, the source boundary can be introduced as:

```text
src/skill/
  mod.rs
  loader.rs
```

The skill content itself belongs to the installed plugin package, not to
Windie's source tree:

```text
research-tools/
  plugin.json
  skills/
    research/
      SKILL.md
```

The same rule applies to future app connectors: their package files belong
inside the plugin that provides them, while Windie's source tree receives an
`src/app/` runtime only when the first app connector needs one. This keeps
Windie a package loader and runtime instead of embedding a catalog of empty or
provider-specific extension implementations.

## What remains code-owned during migration

Until every MCP has migrated, `src/mcp/compatibility.rs` remains a
temporary compatibility registry. It continues to provide the existing
providers and their provider-specific setup behavior.

During the transition, a packaged provider may temporarily replace a
code-owned provider with the same ID in the live registry. Uninstall must
restore the code-owned definition until that provider is fully migrated and
the compatibility registry can be removed.

Do not remove a provider from `compatibility.rs` merely because its package files
exist. Remove it only after the packaged provider passes the complete
installation, discovery, execution, disable, repair, and uninstall tests.

## Migration workflow for each MCP

### 1. Inventory the existing provider

Before writing a package, read the existing provider definition and record:

- stable provider ID and schema prefix;
- display name, description, README, and icon requirements;
- transport: remote HTTP or local stdio;
- command, arguments, working directory, and child environment;
- runtime dependencies such as Node, uv, or a native executable;
- package installation and cache behavior;
- required or optional secrets and their delivery method;
- permissions such as network, filesystem, browser, or external application;
- readiness probes and health checks;
- setup, repair, disable, and cleanup behavior;
- whether an external application must already be running;
- whether uninstall may remove a shared runtime or user-owned resource.

The package manifest describes declarative facts. Provider-specific setup and
cleanup must remain behind a generic runtime boundary until the package system
can express those operations safely.

### 2. Create the plugin package

Use this layout:

```text
packages/<plugin-id>/
  plugin.json
  mcp/server.json
  # local MCPs may also include a .mcpb artifact
  README.md
  assets/icon.svg
```

`plugin.json` describes the installable plugin and points to its components:

```json
{
  "manifest_version": 1,
  "plugin": {
    "id": "example-tools",
    "version": "1.0.0",
    "publisher": "publisher-id"
  },
  "presentation": {
    "name": "Example Tools",
    "description": "A short user-facing description.",
    "readme": "README.md",
    "icon": "assets/icon.svg"
  },
  "components": [
    {
      "type": "mcp",
      "id": "example-tools",
      "manifest": "mcp/server.json",
      "windie": {
        "authentication": {
          "type": "none"
        },
        "permissions": ["network"]
      }
    }
  ]
}
```

The standard MCP Registry `server.json` describes the MCP connection:

```json
{
  "$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
  "name": "com.example/example-tools",
  "title": "Example Tools",
  "description": "A short user-facing description.",
  "version": "1.0.0",
  "remotes": [
    {
      "type": "streamable-http",
      "url": "https://example.com/mcp"
    }
  ]
}
```

Windie-specific authentication, permissions, capabilities, and timeouts stay
in the component's `windie` object in `plugin.json`.

For a local MCP, use the standard package shape. `runtimeHint` describes the
runtime family needed by the generic Windie lifecycle:

```json
{
  "$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
  "name": "com.example/local-tools",
  "title": "Local Tools",
  "description": "A local MCP server.",
  "version": "1.0.0",
  "packages": [
    {
      "registryType": "mcpb",
      "identifier": "https://example.com/releases/local-tools.mcpb",
      "version": "1.0.0",
      "fileSha256": "<sha256>",
      "runtimeHint": "node",
      "transport": {
        "type": "stdio"
      }
    }
  ]
}
```

### 3. Add the marketplace entry

The index is for discovery and distribution metadata. It is not the runtime
source of truth; Windie must still load and validate `plugin.json` from the
installed artifact.

Add one plugin entry and one immutable release:

```json
{
  "index_version": 1,
  "plugins": [
    {
      "id": "example-tools",
      "versions": [
        {
          "version": "1.0.0",
          "components": ["mcp"],
          "capabilities": ["example_search"],
          "manifest_url": "packages/example-tools/plugin.json",
          "artifact_url": "packages/example-tools",
          "digest": "bundled",
          "publisher": "publisher-id",
          "status": "verified"
        }
      ]
    }
  ]
}
```

`digest: "bundled"` is only acceptable for the checked-in development
catalog. A production or remote catalog must use a cryptographic artifact
digest and verify it before installation.

### 4. Connect the manifest to the generic runtime

Do not add a new provider-specific branch to the registry for the migrated
MCP. The desired path is:

1. `PluginStore` loads and validates the installed plugin package.
2. The MCP loader reads the component's standard `server.json`, Windie policy,
   and optional MCPB artifact.
3. The loader creates a generic MCP transport and the registry creates a
   generic `McpToolProvider` from it.
4. The MCP runtime performs initialization and `tools/list`.
5. Windie persists the discovered tool catalog in SQLite.
6. Normal provider and conversation operations use the catalog and registry.

The generic path must preserve the provider's stable ID and schema prefix so
existing tool attachments do not unexpectedly change.

### 5. Preserve secrets and permissions

Secrets belong to Windie's provider configuration boundary, not to package
files. A manifest may declare:

- whether a key is required or optional;
- the stable secret ID;
- where the user can obtain it;
- whether it is delivered as an HTTP Bearer header or a child-process
  environment variable.

The package must never contain a key. Missing optional credentials should use
anonymous access when the MCP supports it. Missing required credentials should
produce a configuration state and an actionable setup URL.

The MCP's declared permissions must flow into the provider manifest and remain
visible to Windie's permission and setup surfaces.

### 6. Test the complete lifecycle

Each migrated MCP needs tests for:

- manifest parsing and invalid-manifest rejection;
- package path traversal rejection;
- package presentation asset validation;
- marketplace index discovery;
- installation into `<plugin-id>/<version>` storage;
- dynamic registry registration;
- tool discovery through the generic MCP provider;
- optional and required credential behavior;
- provider lifecycle persistence;
- disable without deleting the package;
- repair or health-check behavior;
- uninstall and runtime/session cleanup;
- restoration of the temporary code-owned provider, if IDs overlap;
- unchanged conversation attachments unless explicitly mutated by the user.

For remote MCPs, use a local HTTP fixture for protocol tests. Do not make the
normal test suite depend on the live third-party service.

### 7. Remove the old definition last

After the package path is complete:

1. compare the packaged provider manifest with the old provider manifest;
2. verify tool schema names and provider IDs remain compatible;
3. verify setup, health, disable, repair, and uninstall behavior;
4. remove the provider from `compatibility.rs`;
5. remove only provider-specific code that is no longer needed;
6. keep generic MCP transport and lifecycle code;
7. run the complete Rust test suite.

The migration is not complete if the package exists but Windie still needs a
hardcoded definition to know how to start or connect to it.

## Migration order

Use this order because each step proves a harder runtime capability:

| Order | MCP | What it proves |
| --- | --- | --- |
| 1 | Parallel Search | Packaged remote Streamable HTTP, optional API key, dynamic discovery |
| 2 | Desktop Commander | Packaged local stdio, Node dependency, isolated HOME, setup and cleanup |
| 3 | Blender MCP | Pinned uv MCPB plus external Blender bridge requirement |
| 4 | Chrome DevTools | Managed versus existing external browser modes and readiness checks |
| 5 | Bright Data | Bundled Node MCPB with required API token delivered to the child process |
| 6 | CUA Driver | Native macOS MCPB, bundled app identity, and local process lifecycle |

Desktop Commander has completed the local MCPB migration. Its upstream
`@wonderwhy-er/desktop-commander@0.2.47` runtime and dependencies are bundled
inside a verified MCPB. The generic runtime supplies Node, creates its
isolated HOME, writes the blocked-command configuration, discovers tools, and
removes the complete installed version on uninstall. Basic Memory has also
completed migration: its uv MCPB declares the pinned `basic-memory==0.22.1`
runtime, Windie prepares the uv environment before discovery, and uninstall
removes the plugin/runtime while preserving `~/.windie/memory`. Blender MCP
has also completed migration: its MCPB pins `blender-mcp==1.8.3`, Windie
prepares the uv environment, discovers the server's tools, and removes the
installed package and managed uv runtime on uninstall. The external Blender
add-on remains an explicit user setup requirement because Windie does not
silently modify another application's preferences. Chrome DevTools has also
completed migration: its MCPB bundles the pinned npm dependency tree, Windie
manages the Node runtime and browser profile, package metadata supplies the
`list_pages` readiness probe, and mode switching updates the package-owned
launcher without falling back to `npx`. Bright Data has also completed
migration: its pinned `@brightdata/mcp@2.11.1` server and npm dependency tree
are bundled inside a Node MCPB, and the generic package runtime maps the
Windie secret `BRIGHTDATA_API_TOKEN` to the server's `API_TOKEN` environment
variable only at process start. CUA Driver has also completed migration: the
official `cua-driver-rs-v0.12.6` macOS universal release is bundled as a
package-owned MCPB, and Windie launches the app-bundle binary through the
generic stdio runtime with the `mcp` command. The package owns its files and
lifecycle; macOS accessibility and screen-recording permissions remain
operating-system permissions and are not silently granted or revoked by
Windie. The current CUA artifact is macOS-only; other platform artifacts must
be published before advertising CUA Driver on those platforms.

## Acceptance gate for the migration

The package architecture is ready to replace the code-owned MCP system only
when every current MCP can complete this flow through its package:

```text
listed
  -> installed
  -> validated
  -> configured
  -> activated
  -> dynamically discovered
  -> safely used
  -> disabled
  -> repaired
  -> uninstalled
```

Until then, keep the compatibility registry and migrate one provider at a
time. This keeps Windie usable while the package runtime grows without making
the package format responsible for provider-specific unsafe behavior.
