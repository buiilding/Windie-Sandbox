# Blender MCP

Blender MCP lets Windie inspect and control a local Blender instance through a
package-owned MCP server.

## Requirements

- Blender 3.0 or newer.
- The Blender MCP add-on installed and enabled in Blender.
- The Blender MCP panel running its socket bridge on `localhost:9876`.

Windie installs and owns the MCP server, its pinned Python dependency, its uv
cache, its isolated home directory, and its process lifecycle. Windie does not
silently change Blender's preferences or enable an add-on inside the external
Blender application. Follow the upstream add-on setup instructions once, then
repair the plugin in Windie to discover its tools.

The package disables Blender MCP telemetry by default. Blender tools can modify
the open project and can execute Python inside Blender; review tool calls before
allowing changes to scenes or files.

Upstream project: https://github.com/ahujasid/blender-mcp
