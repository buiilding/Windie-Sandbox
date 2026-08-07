# Changelog

All notable changes to Windie are documented here.

This changelog is intentionally curated from the repository history. It records
meaningful product, runtime, and developer-facing changes rather than every
individual commit or internal edit.

## [Unreleased]

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
- Redesigned the Inspector welcome page with a centered hero, clearer
  typography, and a more visible background grid.
- Added welcome-page buttons for exploring extensions and configuring LLM
  providers.
- Removed the combined onboarding overlay and made the welcome page switch to
  a new-chat action after an LLM provider and extension are both ready.
- Added bounded runtime retries for transient model-provider failures, while
  discarding failed partial assistant output and keeping retries invisible in
  the Inspector.
- Added the public release workflow guide for maintainers.
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

- Added `windie-dev` for local development, release testing, and performance
  checks.
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

[Unreleased]: https://github.com/buiilding/Windie-Sandbox/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.0
[0.2.10]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.10
[0.2.9]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.9
[0.2.2]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.2
[0.2.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.0
[0.1.1]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.1.1
[0.1.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.1.0
