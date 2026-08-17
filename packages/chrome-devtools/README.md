# Chrome DevTools

Chrome DevTools lets Windie inspect, debug, and automate an explicitly
selected Chrome browser session through a package-owned MCP server.

Windie installs the pinned Chrome DevTools MCP dependency, its Node runtime,
its isolated HOME, its npm cache, and its managed browser profile.

## Connection modes

Managed mode is the default. Windie starts Chrome DevTools MCP with a separate
persistent profile, so the user's normal Chrome profile is not exposed.

Existing-Chrome mode attaches to an already-running Chrome after the user
enables Chrome's remote-debugging approval flow at:

```text
chrome://inspect/#remote-debugging
```

Browser tools can access websites and perform actions. Keep Windie's approval
mode enabled when reviewing automation.

Upstream project: https://github.com/ChromeDevTools/chrome-devtools-mcp
