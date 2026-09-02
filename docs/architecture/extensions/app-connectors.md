# App connectors

## Purpose

App connectors are intended to let a plugin describe an integration with an
external application. Windie does not currently implement the connection
runtime for them. At the moment, an app component is metadata that can be
validated and shown in the plugin index; it is not a connected application or
an executable model capability.

## Owns

- Recognizing a plugin component with `"type": "app"`.
- Validating its JSON app manifest, which currently contains optional `name`
  and `description` fields.
- Loading that metadata from an installed plugin without connecting to the
  external application.
- Including the app's ID, name, purpose, and `installed` state in the
  installed-plugin summary and compact plugin index.

## Does not own

- External-application connections, API clients, OAuth, API keys, or other
  authentication and authorization flows.
- A runtime protocol or process for an app connector.
- Tool discovery, tool execution, or model-facing executable schemas. MCP
  components own that capability boundary today.
- User approval, permissions, or conversation state.

## Main flow

1. A plugin may declare an app component and point to its JSON manifest.
2. Windie validates the component and reads its name and description while
   loading the installed plugin.
3. The plugin catalog projects the app as metadata in the installed-plugin
   summary and model-facing plugin index.
4. The flow currently stops there. Windie does not establish a connection,
   request authorization, or make an app capability available to the runtime.

## Important invariants

- An app entry in a plugin manifest means only that metadata was declared and
  validated; it does not mean the external application is connected.
- App metadata does not create a tool schema or allow the model to call the
  external application.
- There is currently no app-connector authentication, authorization, runtime
  boundary, or end-to-end app-connector test flow in Windie.
- Any future connector implementation should keep external authorization
  explicit and user-controlled, and should keep secrets outside conversation
  history.

## Related code

- [`src/plugin/manifest.rs`](../../../src/plugin/manifest.rs) — `AppManifest`
  and the `PluginComponentKind::App` manifest contract.
- [`src/plugin/store.rs`](../../../src/plugin/store.rs) — validates app
  manifests and loads app metadata without connecting to an external app.
- [`src/plugin/catalog.rs`](../../../src/plugin/catalog.rs) — projects app
  metadata into `AppSummary` and the compact plugin index.
- [`src/plugin/mod.rs`](../../../src/plugin/mod.rs) — plugin package tests and
  the public plugin boundary.
