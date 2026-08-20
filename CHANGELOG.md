# Changelog

This is Windie's detailed engineering changelog. It records meaningful
product, runtime, documentation, and developer-facing changes with enough
context to preserve the project's history. Concise public release highlights
are maintained separately in [`RELEASE_NOTES.md`](RELEASE_NOTES.md).

## [Unreleased]

- Excluded the `local-mcp-fixture` test package from generated marketplace
  releases while retaining it for package-installation tests.
- Moved installed-runtime lifecycle instructions from the README into the CLI
  command reference and clarified how to invoke the development CLI from a
  checkout.
- Added a complete `Backend.md` inventory for every Rust source file and
  corrected stale module-boundary references.
- Clarified that the generated plugin capability index is separate runtime
  context and remains present when a user replaces or clears a conversation
  system prompt.
- Consolidated repository development, release, marketplace, and benchmark
  workflows into the public `windie` CLI, removing the separate development
  executable and activation scripts.
- Added a durable database-wide session event stream with replayable numeric
  cursors for API and CLI clients.
- Tightened SQLite-backed session execution claims with a unique fencing token
  for every run, so a cancelled runner cannot write through a later claim from
  another API or CLI execution of the same session.
- Routed API and CLI work through one `execute_session` workflow and replaced
  the separate normal, selected-head, and approval claim functions with one
  typed claim entry point.
- Made claimed user/assistant/tool-result insertion, session-head advancement,
  and applicable durable replay-event creation one SQLite transaction, with
  persistence failures returned to the running session and rollback behavior
  covered by tests.
- Removed the runtime's production direct-message persistence path and the
  duplicate CLI message saver, leaving session execution with one required
  persistence contract.
- Centralized final model-context construction so execution, inspection, and
  input-token counting use the same plugin index and built-in tool schemas.
- Refreshed Inspector provider installations and available tool schemas after
  plugin install or uninstall, removing the need for a page reload.
- Moved public CLI command handlers out of `src/main.rs` into domain-specific
  `cli::adapter` modules, leaving the binary entrypoint responsible only for
  parsing and dispatch wiring.
- Unified CLI and API session runtime setup and event persistence, including
  cancellation events and CLI failure recording.
- Split the MCP runtime into protocol, stdio, session, and transport modules
  behind a small compatibility facade, preserving existing MCP callers while
  making process I/O and session lifecycle independently inspectable.
- Removed the legacy code-owned Parallel Search MCP fallback. MCP providers
  now come from installed plugin packages.
- Fixed API startup by keeping the blocking marketplace HTTP client inside a
  blocking worker instead of creating or dropping it from the Tokio runtime.
- Simplified the Inspector plugin detail page by hiding component lifecycle
  controls and redundant installation/version labels while preserving backend
  plugin installation behavior.
- Refined the Inspector extensions catalog with an inset search field and
  collapsible Installed and Recommended sections.
- Added the unified model-facing plugin index. Each turn now derives installed
  plugin metadata from local packages, available plugin metadata from the
  marketplace snapshot, and nested MCP lifecycle state from SQLite without
  persisting the index or exposing MCP schemas in the prompt.
- Restored the temporary `windie__read_skill` and `windie__attach_mcp`
  compatibility controls, and removed the provider-listing and provider-
  attachment built-ins. Installed plugin metadata and MCP schemas remain
  available through the package and provider runtime paths.
- Migrated Windie's MCP extension foundation from code-owned provider
  definitions to versioned packaged plugins. Added plugin manifests,
  marketplace index and artifact installation, standard MCP `server.json`
  metadata, MCPB local packages, declarative runtime setup, package-owned
  lifecycle cleanup, and marketplace API routes.
- Migrated the checked-in MCP providers, including Parallel Search, Desktop
  Commander, Blender, Bright Data, Chrome DevTools, CUA Driver, and Basic
  Memory, into package fixtures and removed their code-owned definitions.
- Updated the Inspector to treat plugins as the marketplace and installation
  unit, while nesting MCP enablement, repair, credentials, and tool runtime
  controls under each plugin's components.
- Added local marketplace serving and end-to-end package lifecycle coverage for
  remote MCPs and local MCPB runtimes.
- Retained the checked-in marketplace index and package fixtures as a
  deterministic development and test fallback while production continues to
  use the hosted marketplace.
- Documented that future skill and app runtime modules should be added only
  when implemented, while their `SKILL.md` and connector files remain inside
  installed plugin packages.
- Reorganized the Rust source tree so plugin storage, MCP runtime, tool
  execution, managed runtimes, local process control, and model runtime code
  have separate module boundaries; removed the old `tool_provider` module.

## [0.3.2] - 2026-08-10

- Updated the runtime for the current Rust Clippy checks used by release CI.
- Serialized environment-sensitive tests so the default parallel test run is
  reliable in CI.
- Added separate concise public release notes while retaining detailed
  engineering history in this changelog, with CI validation for both files.
- Updated the Inspector submodule to merged commit `3509b03` in preparation for
  the next release.
- Replaced the Inspector's placeholder square branding with the circular Windie
  icon in the browser favicon and top bar.
- Moved LLM Providers above Extensions in the Inspector activity bar.
- Changed the Inspector transcript to scroll to the bottom when switching
  conversations while preserving the user's position during new messages and
  streaming updates.
- Removed the Chrome remote-debugging server check from existing-Chrome MCP
  setup; users now confirm the Chrome setting directly before installation or
  reconfiguration starts.
- Fixed the existing-Chrome setup link so Windie opens Chrome's remote-
  debugging settings through the local backend instead of blocked page script.
- Closed the Chrome ownership dialog immediately after existing-Chrome
  confirmation so provider setup can continue in the extension status view.
- Restored a TCP-only `127.0.0.1:9222` check for existing Chrome, skipping the
  settings instructions when debugging is already enabled and polling after
  Windie opens Chrome's settings page.
- Clarified message actions with a dedicated branch label/icon and a separate
  fork icon.
- Added Chrome DevTools MCP browser ownership selection: install and configure
  can use a Windie-managed profile or an explicitly approved existing Chrome,
  with remote-debugging preflight, MCP readiness validation, and safe mode
  switching without reinstalling the package.
- Added a full-access action to the inline Inspector tool-approval prompt so
  approval-waiting sessions can switch to automatic approval from the prompt.
- Updated the README Discord badge to use the active Windie community invite.
- Fixed Basic Memory provider uninstall when Windie's project is the CLI
  default, stopped new Windie projects from changing the global Basic Memory
  default, and preserved provider command diagnostics written to stdout.
- Aligned the landing design guidelines, navigation lockup, CTA copy, and
  README headline hierarchy with the current Windie identity.
- Updated the landing-page marketing identity PRD and clarified the Desktop
  Commander capability preview in the README.
- Added light and dark Windie app icon assets under `assets/branding/`.
- Added real provider uninstall cleanup: active MCP sessions stop first,
  provider-owned caches/configuration and secrets are removed, CUA Driver uses
  its official purge uninstaller, and shared Node/uv runtimes are retained
  while another installed provider needs them. Known limitation: Windie does
  not yet persist ownership for pre-existing CUA installations or Basic Memory
  project registrations, so uninstall may remove those resources if they are
  configured through Windie's provider boundary.
- Added Windie-managed provider README content and a SQLite-backed provider
  tool catalog. Provider setup and health checks refresh the catalog, while
  tool listings and extension views read the persisted schemas without starting
  providers.
- Handled new-conversation attempts without configured LLM keys as setup guidance instead of an uncaught runtime error.
- Start the Inspector with the sidebar collapsed while keeping the activity bar visible.
- Let activity-bar buttons toggle the Inspector sidebar like VS Code.
- Removed the orange hover highlight from the Inspector sidebar resize handle.
- Constrained Basic Memory's LiteLLM dependency below 1.92 during provider
  setup and launch to avoid unsupported local Rust/maturin builds on macOS.
- Added regression coverage for Basic Memory's runtime and package-preparation
  LiteLLM constraints.
- Cleaned up Inspector sidebar behavior with reliable menu dismissal, consistent
  graph/provider headers, and constrained provider API-key inputs.
- Fixed Streamable HTTP MCP tool execution by using async HTTP sessions and
  per-provider async pooling, preventing Parallel Search calls from panicking
  inside Tokio workers and leaving sessions stuck as running.
- Fixed the landing page mobile menu by making the open navigation an opaque,
  full-height overlay with separated links, a full-width install CTA, and
  background scroll locking.

## [0.3.1] - 2026-08-07

- Prevented stale model-catalog refresh requests from overwriting newer
  results, while preserving the last known-good catalog during transient
  provider failures.
- Added a Motion Primitives-style shimmer effect to the live Inspector thinking
  label, with reduced-motion support and muted theme-aware text.
- Migrated the Inspector animation dependency from Framer Motion to the
  current Motion package for future animated components.
- Added an animated thinking orb to the Inspector reasoning lane while the
  assistant is actively processing.
- Prefer GPT-5.6 Luna, Kimi K3, then Claude Sonnet 4.5 as Windie's initial
  models when they are available, with catalog fallback for other providers.
- Added built-in Kimi Code provider support through Bifrost, including API-key setup,
  model discovery, streaming chat, and tool use.
- Pinned the reviewed Kimi Code gateway integration to its merged Bifrost
  stable-branch commit.
- Redesigned the Inspector welcome page with a centered hero, clearer
  typography, and a more visible background grid.
- Added welcome-page buttons for exploring extensions and configuring LLM
  providers.
- Removed the combined onboarding overlay and made the welcome page switch to
  a new-chat action after an LLM provider and extension are both ready.
- Added bounded runtime retries for transient model-provider failures, while
  discarding failed partial assistant output and keeping retries invisible in
  the Inspector.
- Added the public release workflow guide for maintainers, including separate
  stable-branch and vendor-PR handling for Bifrost and the Inspector.
- Let the Inspector manage structured Bifrost model providers, including
  custom and local providers.
- Documented the public release workflow, including versioning, checks,
  submodule pins, pull requests, tags, and release verification.
- Added native Streamable HTTP MCP transport support with persistent sessions,
  SSE/JSON responses, 404 recovery, and safe credential handling.
- Added the curated Parallel Search MCP provider with optional Bearer
  authentication.

## [0.3.0] - 2026-08-06

- Added Chrome DevTools MCP as an extension for browser use.
- Added a clearer extension setup flow.
- Added an activity bar for conversations, the conversation tree, extensions,
  model providers, and settings.
- Added sidebars with conversation and extension lists.
- Added extension READMEs and available tools.
- Fixed the previous Continue and Query button combination in the composer.
- Improved local activation scripts for zsh developers.
- Fixed development activation when the build fails.
- Fixed release workflows so release notes load correctly.

## [0.2.9] - 2026-08-05

- Added repository development utilities for local development, release
  testing, and performance checks.
- Added performance tracking to help catch slowdowns.
- Added custom ports so multiple Windie installs can run side by side.
- Added `windie -v` and simpler help output.
- Added easier commands for starting, stopping, and checking Windie locally.
- Moved the Inspector into its own versioned component, making updates and
  releases more reliable.
- Combined Continue and Query into one composer action.
- Windie no longer forces default models.
- Moved tool approval prompts above the message box.
- Improved the welcome screen, theme defaults, conversation controls, and
  overlays.
- Added clearer setup and developer documentation.
- Fixed Inspector builds in CI and public releases.
- Improved macOS release compatibility.
- Fixed local release testing so it can find the installer.

## [0.2.0] - 2026-07-30

Issue #26 delivers cross-platform installation, bundled gateway reliability,
and provider lifecycle diagnostics for fresh macOS, Linux, and Windows setup.

### Installation and providers

- Added user-local Node.js and uv runtime provisioning with checksum
  verification and platform-specific executable resolution.
- Added Windows `.exe`, `.cmd`, and PowerShell handling, including CUA's
  official Windows installer path.
- Added actionable provider readiness states for missing runtimes, external
  applications, permissions, secrets, unsupported platforms, and repairable
  failures.
- Added provider-specific preflight checks for Blender and Bright Data while
  preserving shared runtimes across provider uninstall operations.

### Runtime and release reliability

- Hardened bundled Bifrost startup, stale-process recovery, PID ownership,
  port diagnostics, and release-manifest compatibility checks.
- Added checksum and manifest validation to the macOS, Linux, and Windows
  installation scripts.
- Expanded release packaging and CI to Linux, macOS, and Windows x64 assets.

### Inspector and onboarding

- Added model-provider management to the Inspector, including provider status,
  API-key creation, key deletion, and clearer handling of invalid keys.
- Added automatic onboarding when no usable model provider is configured.
- Simplified the Inspector surface by removing the separate setup control,
  system-scope labels, and the extension-library header.
- Improved provider ordering and status presentation so configured providers
  are easier to identify and providers requiring structured setup remain
  separate from ordinary API-key providers.

### Runtime and session architecture

- Made session-branch resolution authoritative in the backend instead of
  inferring session ownership from frontend state.
- Solidified the conversation-path and session-event model, including session
  reuse when selecting an existing conversation head.
- Split frontend conversation, session, and resource-catalog responsibilities
  into clearer state boundaries.
- Separated runtime orchestration from terminal/output presentation.
- Removed the obsolete `windie-ui` client in favor of the Inspector.

### Project structure

- Added and expanded the backend and frontend ownership guides so the codebase
  documents its module boundaries directly.
- Fixed the release workflow’s repository scope.

## [0.1.1] - 2026-07-28

Updated the application version, public documentation, and release packaging.

### Documentation and distribution

- Corrected the local API examples to use the current Inspector address and
  session query route.
- Published release assets for macOS and Linux on ARM64 and x86_64.
- Aligned the installed binary version with the `v0.1.1` release tag.

## [0.1.0] - 2026-07-26

The first tagged Windie release: a local AI runtime with a browser Inspector,
conversation-tree context control, model-provider integration, and extensible
tool providers.

### Runtime foundations

- Established conversations as durable message trees with explicit paths,
  branching, editing, truncation, removal, and forking.
- Introduced durable sessions as executable branches over the shared
  conversation tree.
- Added streamed session events, session recovery, queued inputs, tool
  approvals, cancellation, and runtime state persistence.
- Added model-context inspection, token accounting, compaction checkpoints, and
  performance benchmarks for core runtime operations.
- Organized the backend around explicit ownership boundaries for conversation,
  session, runtime, storage, tools, providers, API, CLI, and output.

### Model providers

- Integrated Windie with the Bifrost OpenAI-compatible gateway for provider
  routing and model discovery.
- Added provider model listing, Responses API requests, streamed responses,
  reasoning metadata, token usage, model parameters, and image input support.
- Added terminal and Inspector onboarding for configuring model providers.

### Tools and extensions

- Introduced a provider-backed tool layer with a shared model-facing schema.
- Added approved MCP providers for CUA Driver, Desktop Commander, Blender,
  Bright Data, and Basic Memory.
- Added provider manifests, installation and lifecycle state, setup secrets,
  persistent MCP sessions, tool discovery, approval policy, and bounded tool
  execution.
- Added the Inspector’s extension catalog and provider installation controls.

### Inspector and local development

- Added the browser-based Windie Inspector for inspecting conversations,
  sessions, model context, tools, approvals, and streamed assistant output.
- Added a localhost HTTP API with authentication, SSE session events, and
  static Inspector serving.
- Added local setup commands, database recreation tooling, runtime benchmarks,
  and development documentation.

### Distribution

- Added the one-line installer and bundled Bifrost into the local installation
  path.
- Added release packaging and GitHub Actions publishing support.
- Added the Windie wordmark, Inspector and extension previews, and initial
  project documentation.

[Unreleased]: https://github.com/buiilding/Windie-Sandbox/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.2
[0.3.1]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.1
[0.3.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.0
[0.2.10]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.10
[0.2.9]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.9
[0.2.2]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.2
[0.2.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.0
[0.1.1]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.1.1
[0.1.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.1.0
