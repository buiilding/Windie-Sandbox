# Awesome Mineflayer MCP

Awesome Mineflayer MCP gives Windie control over a Minecraft Java bot through
the upstream `awesome-mineflayer-mcp` server version `1.3.2`. It exposes bot
lifecycle, observation, movement, mining, building, crafting, inventory,
combat, chat, and waypoint tools.

## Connect a bot

This package does not connect a bot automatically. After installation, use the
upstream `connect_bot` tool with the target host, port, username, and
authentication mode. For a local offline-mode test server, use `localhost`,
port `25565`, an offline username, and `auth: "offline"`. Online-mode servers
require Microsoft authentication and the upstream device-code flow.

Keep Windie's manual approval mode enabled. Minecraft tools can mutate the
world, move the bot, interact with entities, use inventory items, and send
chat or slash commands. The advanced raw protocol tool group is disabled by
this package.

## Bundled runtime

The MCPB bundles the upstream production build and required Node dependencies.
Windie supplies its managed Node runtime, so a system Node installation is not
required for installed use. The optional upstream screenshot renderer is not
bundled to keep the package practical to distribute; all core bot controls and
the dependency-free map renderer remain available.

Upstream project: https://github.com/G0Osey99/awesome-mineflayer-mcp
