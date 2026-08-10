# Desktop Commander

Desktop Commander lets Windie read, write, and manage local files and processes through MCP.

## What it provides

The provider exposes filesystem and local-process tools. Windie runs it with an isolated configuration and telemetry disabled.

## Requirements

- Node.js and `npx`.
- Review the filesystem and process permissions before attaching tools.

## Safety

Filesystem and process tools can change local state. Use manual approval when actions are destructive or difficult to undo.
