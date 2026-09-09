# Marketplace

## Purpose

The marketplace is Windie-owned. Windie creates its own marketplace to host
the plugins that Windie supports.

Windie is the runtime, while the marketplace provides the capabilities that
can be added to Windie. This keeps the runtime focused and lets developers add
new packages quickly instead of hard-coding every capability into Windie.

## Owns

The marketplace is built from packages in `packages/<plugin-id>/` that opt into
publication with `"marketplace": { "publish": true }` in `plugin.json`. It
creates a versioned `.tar.gz` archive for each package and generates an
`index.json` catalog containing the plugin ID, version, components,
capabilities, publisher, presentation metadata, archive URL, and SHA-256
digest.

The source packages are stored in Windie's GitHub repository. For production,
the generated archives are uploaded to GitHub Releases, while the catalog
site is deployed to Vercel and exposed at
[`https://marketplace.windieos.com/index.json`](https://marketplace.windieos.com/index.json).
The API reads this index and gives the catalog to the Inspector so users can
see plugins and install one based on their workflow.

## Does not own

The marketplace does not execute plugins, start MCP processes, connect app
connectors, or own the local installed-plugin state. Windie's plugin store,
component lifecycle, MCP runtime, and approval policy handle those parts.

The marketplace is a catalog and distribution boundary. Seeing a plugin in
the catalog does not by itself run it or bypass Windie's permission
boundaries.

## Main flow

1. A developer adds a package under `packages/` and marks it for marketplace
   publication.
2. The marketplace workflow validates the package, creates its versioned
   `.tar.gz` archive, calculates its SHA-256 digest, and writes `index.json`.
3. Publishing uploads the archives to a GitHub Release and deploys the catalog
   site. Local development can build or serve the same catalog through
   `windie marketplace build` or `windie marketplace serve`.
4. The API fetches the index, and the Inspector displays the available plugins.
   When a user installs one, Windie downloads the listed archive, verifies
   its digest, validates its manifest, and stores the versioned plugin locally.

## Important invariants

- `index.json` is generated from the package manifests and archive contents. It
  is not edited by hand because its metadata and digests must match the
  published archives.
- Only packages that explicitly opt into marketplace publication are included
  in the catalog.
- Plugin releases are versioned, and Windie verifies the declared SHA-256
  digest before installing an archive.
- Marketplace discovery does not automatically execute a plugin or bypass
  Windie's component lifecycle and approval policy.

## Related code

- [`src/plugin/catalog.rs`](../../../src/plugin/catalog.rs) — validates the
  versioned index and projects marketplace and installed-plugin summaries.
- [`src/plugin/installer.rs`](../../../src/plugin/installer.rs) — downloads,
  verifies, and installs marketplace archives.
- [`src/api/plugin.rs`](../../../src/api/plugin.rs) — exposes marketplace
  discovery and plugin installation routes to the Inspector.
- [`src/dev.rs`](../../../src/dev.rs) — builds, serves, and publishes the
  marketplace catalog and archives.
