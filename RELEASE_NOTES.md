# Release notes

Short, user-facing highlights for Windie releases. This file is the source
for GitHub release descriptions; the detailed engineering history is kept in
[`CHANGELOG.md`](CHANGELOG.md).

## [Unreleased]

- No unreleased changes yet.

## [0.3.2] - 2026-08-10

- Refreshed the Inspector's branding and improved the layout of its activity
  bar.
- Improved conversation reading and navigation, including more reliable
  scrolling and clearer branch and fork actions.
- Added clearer and safer Chrome DevTools setup for connecting to an existing
  Chrome profile.
- Improved extension description, setup, provider cleanup, and tool discovery.
- Added clearer guidance when a model provider still needs to be configured.
- Improved the landing page, community link, and release presentation.
- Improved release communication with concise public notes alongside the
  detailed engineering changelog.

## [0.3.1] - 2026-08-07

- Added support for configuring structured, custom, local, and Kimi Code model
  providers from the Inspector.
- Improved model-provider reliability, including safer refreshes and retries
  when a provider is temporarily unavailable.
- Added clearer onboarding and welcome-screen actions for models and
  extensions.
- Added animated thinking indicators to make active model work easier to see.
- Added Streamable HTTP MCP support and the Parallel Search extension.
- Improved release and developer documentation.

## [0.3.0] - 2026-08-06

- Added the browser-based Inspector activity bar and sidebars for conversations,
  the conversation graph, extensions, model providers, and settings.
- Added Chrome DevTools as an extension with clearer setup guidance.
- Added extension documentation and tool visibility in the Inspector.
- Improved the composer, local activation scripts, and release workflows.

## [0.2.9] - 2026-08-05

- Added `windie-dev` for local development, release testing, and performance
  checks.
- Added easier commands for starting, stopping, and checking Windie locally.
- Improved the welcome screen, theme defaults, conversation controls, and tool
  approval prompts.
- Improved release compatibility across macOS, Linux, and the Inspector.

## [0.2.0] - 2026-07-30

- Added cross-platform installation and bundled gateway reliability for macOS,
  Linux, and Windows.
- Added model-provider management, API-key setup, and automatic onboarding in
  the Inspector.
- Added provider readiness checks, installation lifecycle controls, and safer
  cleanup for extensions.
- Improved conversation branching, session recovery, queued inputs, approvals,
  and model-context inspection.
- Expanded release packaging and CI for macOS, Linux, and Windows.

## [0.1.1] - 2026-07-28

- Updated the application version and release packaging.
- Published ARM64 and x86_64 release assets for macOS and Linux.
- Updated local API documentation for the current Inspector endpoints.

## [0.1.0] - 2026-07-26

The first Windie release introduced a local AI runtime with a browser Inspector,
conversation-tree context control, model-provider integration, and extensible
tool providers.

- Added durable conversations, branching, sessions, streamed events, queued
  inputs, approvals, and cancellation.
- Added Bifrost model routing, model discovery, streaming responses, reasoning,
  token usage, and image input.
- Added approved MCP extensions, provider installation, tool discovery, and
  permission-aware execution.
- Added the localhost API, local setup commands, the one-line installer, and
  release packaging.

[Unreleased]: https://github.com/buiilding/Windie-Sandbox/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.2
[0.3.1]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.1
[0.3.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.3.0
[0.2.9]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.9
[0.2.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.2.0
[0.1.1]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.1.1
[0.1.0]: https://github.com/buiilding/Windie-Sandbox/releases/tag/v0.1.0
