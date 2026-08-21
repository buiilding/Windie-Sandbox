# Bright Data

Bright Data provides live web search, scraping, browser, and web-data tools
through a local MCP server backed by Bright Data’s cloud APIs.

Windie installs the pinned Bright Data MCP package and its Node dependencies
inside the plugin artifact. It does not use `npx`, download npm packages when
the server starts, or run marketplace-provided shell scripts.

## Setup

Create a Bright Data API token at [brightdata.com](https://brightdata.com/),
then configure `BRIGHTDATA_API_TOKEN` in Windie. Windie passes that stored
secret to the local server as `API_TOKEN` only while the server is running.

Bright Data may create or use the configured `mcp_unlocker` and `mcp_browser`
zones when its server starts. Those are Bright Data account-side effects and
are controlled by the Bright Data service, not by Windie’s installer.
