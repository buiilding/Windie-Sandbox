# Plugins

## Purpose

Plugins are Windie's installable package boundary for optional capabilities.
They are packages that can be published and installed for use in a production
Windie runtime.

A plugin can contain app connectors, skills, MCPs, or any combination and
number of those components. This lets Windie add capabilities without putting
every integration directly into the runtime.

## Owns

Every plugin has a top-level `plugin.json` manifest. The manifest defines the
plugin ID, version, publisher, user-facing presentation metadata, and its
component list. Each component has its own type, ID, and manifest path.

The supported component types are:

- `mcp` — an MCP server that exposes tools through a local process or hosted
  endpoint.
- `skill` — instructions that Windie can load when the model needs them.
- `app` — app-connector metadata. The current implementation validates and
  indexes this metadata, but does not yet provide a complete app connection
  runtime.

The current checked-in packages under `packages/` all use an `mcp` component.
The manifest format supports `skill` and `app` components as well, including
plugins that combine multiple component types.

The package format is general enough for different component combinations, but
some fields are Windie-specific. For example, MCP components can declare
Windie permissions, capabilities, authentication, setup files, environment
values, local artifacts, and timeout limits. Windie validates the manifest and
component files before storing an installed plugin in its versioned local
plugin store.

## Does not own

The plugin package does not itself execute tools, run skill instructions, or
connect to an external app. The MCP runtime, skill loader, and future app
connector runtime own those behaviors.

Plugins also do not decide whether a tool call is allowed. Windie's tool policy
and conversation approval flow still apply after a plugin is installed and its
components are registered.

## Main flow

1. Windie discovers a plugin from the marketplace catalog or a bundled package.
2. Windie reads `plugin.json`, validates the plugin identity and component
   references, then validates each component manifest and required package
   file.
3. Windie copies the validated package into its versioned local plugin store
   and registers the component providers available to the runtime.
4. The plugin catalog projects the installed plugin and its component metadata
   into the generated runtime plugin index. The model can then discover the
   available capability and deliberately load a skill or attach an MCP when
   needed.

`index.json` is the marketplace index, not the full runtime tool list. It gives
the API and Inspector discovery metadata such as plugin versions, component
types, presentation data, artifact URLs, and digests. The runtime plugin index
is generated from the installed-plugin store and the current marketplace
snapshot for model-facing discovery.

## Important invariants

- Plugin manifests are typed and versioned. A plugin must contain at least one
  component, and component IDs must be unique within the plugin.
- Installed packages are stored by plugin ID and version. Marketplace archive
  installation verifies the release digest and confirms that the package
  manifest matches the marketplace release identity.
- Installing a plugin registers its declared components, but does not remove
  Windie's permission, health, attachment, or approval boundaries.
- The marketplace index and the model-facing runtime plugin index are separate:
  the marketplace index describes distributable releases, while the runtime
  index describes capabilities currently installed on this machine.

## Related code

- [`src/plugin/manifest.rs`](../../../src/plugin/manifest.rs) — typed plugin,
  component, presentation, Windie policy, and manifest validation contracts.
- [`src/plugin/store.rs`](../../../src/plugin/store.rs) — versioned local
  plugin storage and archive installation.
- [`src/plugin/catalog.rs`](../../../src/plugin/catalog.rs) — marketplace and
  installed-plugin summaries plus the model-facing runtime index.
- [`src/plugin/installer.rs`](../../../src/plugin/installer.rs) — marketplace
  archive download and digest verification.
- [`src/mcp/loader.rs`](../../../src/mcp/loader.rs) — loads MCP components from
  installed plugin packages.
