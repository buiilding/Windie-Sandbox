# Windie documentation

This is the entry point for documentation about developing, operating, and
understanding Windie.

## Develop and contribute

- [Develop Windie from source](guides/development/README.md) — platform setup,
  checkout bootstrap, verification, and the local gateway, API, and Inspector.
- [Contributing](../CONTRIBUTING.md) — issue, branch, testing, changelog, and
  pull-request expectations.

## Source maps

- [Backend source map](index/Backend.md) — Rust runtime and CLI inventory.
- [Frontend source map](index/Frontend.md) — Inspector application inventory.
- [Command reference](index/commands.md) — concrete Windie CLI commands.

## Understand the runtime

- [Architecture overview](architecture/overview.md) — start here for the
  runtime's durable state, execution, model access, extensions, interfaces,
  and independent local components.
- [Architecture references](architecture/README.md) — detailed explanations by
  runtime responsibility.
- [Architecture decisions](decisions/) — accepted design decisions and their
  rationale.

## Use and operate Windie

- [Inspector access](guides/hosted-inspector.md) — local and hosted Inspector
  browser boundaries.
- [Desktop notifications](guides/desktop-notifications.md) — notifier behavior
  and development testing.
- [Plugin packages](guides/plugin-packages.md) — package creation, local
  marketplace testing, and publication.
- [Release process](../RELEASING.md) — release verification and packaging.
