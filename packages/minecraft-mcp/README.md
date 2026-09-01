# Minecraft MCP

Minecraft MCP lets Windie control a bot in a locally hosted Minecraft Java
world. It bundles upstream Minecraft MCP Server `2.0.4` from commit
`a7f1fb4c71f535e04f41ab9b51fcf8e4243261f3` with its production Node
dependencies.

## Before setup

Start Minecraft Java, open the intended single-player world to LAN on port
`25565`, and keep it running. This first package connects only to the upstream
defaults:

```text
host: localhost
port: 25565
bot username: LLMBot
```

Windie does not open a world to LAN or launch Minecraft. The server attempts
to connect as soon as it starts, including during MCP tool discovery. If the
world is not available at those defaults, setup or repair will fail. Do not
leave the LAN port automatic: Minecraft may choose a different port.

The upstream release states support for Minecraft Java `1.21.11`; newer game
versions may not work until the upstream server is updated.

Minecraft tools can move the bot, change blocks, use inventory items, and send
chat messages. Keep Windie's manual approval mode enabled while using it.

Upstream project: https://github.com/yuniko-software/minecraft-mcp-server
