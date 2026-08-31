# Plugin packages

## Purpose

This guide describes how to add an installable plugin under `packages/`, test
the generated marketplace locally, and publish a marketplace release.

A plugin is the package and marketplace unit. An MCP server, skill, or app
connector is a component inside that package.

## Package layout

Start with a new directory at `packages/<plugin-id>/`. An MCP-only plugin
normally contains:

```text
packages/<plugin-id>/
├── plugin.json
├── README.md
├── assets/
│   └── icon.svg
└── mcp/
    ├── server.json
    └── <optional local-server>.mcpb
```

`plugin.json` is Windie's outer manifest. It supplies a stable plugin ID,
version, publisher, presentation metadata, a component list, and the explicit
marketplace opt-in:

```json
"marketplace": { "publish": true }
```

For an MCP component, its `manifest` points to standard MCP `server.json`.
The component's nested `windie` metadata holds Windie-specific policy such as
declared permissions, capabilities, authentication delivery, timeouts, and
local MCPB setup. Keep standard MCP metadata in `server.json` and Windie policy
in `plugin.json` so the server description remains reusable by other MCP
clients.

Skills use a `SKILL.md` component, and app connectors use an app component.
The manifest format supports both, even though the current public packages are
MCP-based.

## Add and validate a package

1. Copy the smallest comparable package, such as `packages/parallel-search`
   for a hosted HTTP MCP or `packages/desktop-commander` for a local MCPB.
2. Choose a unique plugin ID and component ID. Do not install both a direct
   provider and a plugin component with the same provider ID; they can collide
   in the tool registry.
3. Add a concise README and icon. Package presentation metadata is what the
   Inspector uses to describe the plugin before and after installation.
4. Declare only the secrets and permissions the provider actually needs. Never
   place credentials in `plugin.json`, `server.json`, a README, or an archive.
5. Build the local marketplace:

   ```text
   cargo run --bin windie -- marketplace build
   ```

   This discovers every opted-in package, validates it, creates a versioned
   `.tar.gz` archive, calculates its SHA-256 digest, and writes a generated
   catalog to `target/local-marketplace/index.json`.
6. To exercise installation through the same marketplace path, serve that
   catalog and start Windie with its index URL:

   ```text
   cargo run --bin windie -- marketplace serve
   WINDIE_MARKETPLACE_INDEX_URL=http://127.0.0.1:8788/index.json windie api start
   ```

   The API loads the marketplace, the Inspector can install the plugin, and
   normal component setup discovers its tools before the model can attach them.

## Publish a marketplace release

Publishing changes external state. Run it only from reviewed release-ready
source with authenticated GitHub CLI and Vercel CLI sessions:

```text
cargo run --bin windie -- marketplace publish
```

The command creates immutable archives, uploads them to an automatically named
GitHub Release, then deploys the catalog site to the `windie-marketplace`
Vercel project. The catalog points at the GitHub Release assets and includes
their digests. The resulting production catalog is:

```text
https://marketplace.windieos.com/index.json
```

Do not hand-edit a generated marketplace `index.json`: its artifact URLs and
digests must match the generated archives.

## Related code

- `src/plugin/manifest.rs`: package and component manifest validation.
- `src/plugin/store.rs` and `src/plugin/installer.rs`: verified installation.
- `src/mcp/loader.rs`: turns installed MCP package components into transports.
- `src/dev.rs`: marketplace build, local serving, and publication.
- `docs/skills/migrate-code-owned-mcps.md`: detailed migration reference.
