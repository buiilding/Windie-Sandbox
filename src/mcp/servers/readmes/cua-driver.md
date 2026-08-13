# CUA Driver

CUA Driver lets Windie operate the local computer through a computer-use MCP server.

## What it provides

The provider exposes computer-control tools to an AI session. Windie only makes those tools available to a conversation when they are explicitly attached.

## Requirements

- CUA Driver installed locally.
- The upstream CUA Driver skill pack installed into Windie's plugin store.
- Computer-control permissions granted to the required applications.

Windie installs the executable first, then runs `cua-driver skills install
--all-platforms` and materializes the returned skill pack as the installed
`cua-driver` plugin. The MCP process and its tool schemas remain unavailable
to conversations until the plugin is explicitly attached.

## Safety

Computer-control tools can interact with the desktop and external applications. Keep approval mode enabled when reviewing actions before execution.
