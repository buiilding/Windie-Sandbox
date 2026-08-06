# Chrome DevTools MCP

Windie provides the official Chrome DevTools MCP as the `chrome-devtools`
provider.

## First-version behavior

- Windie launches `chrome-devtools-mcp@1.6.0` through `npx` over stdio.
- Windie uses the normal, full Chrome DevTools MCP tool catalog. It does not
  pass `--slim`.
- Windie launches a separate Chrome profile at:

  ```text
  ~/.windie/mcp/chrome-devtools/profile/
  ```

- Cookies, local storage, and browser state in that profile persist when the
  provider is disabled. The profile is not deleted by disable or uninstall.
- Users log into websites once in the Windie-managed profile and Windie reuses
  that session in later runs.
- Usage statistics and MCP update checks are disabled for this provider.
- Provider installation verifies the MCP catalog before the provider is
  enabled. Chrome itself may start lazily when the first browser tool runs.
- An explicit provider health check runs the safe `list_pages` probe to verify
  Chrome/profile readiness without navigating or changing page state.

## Safety and scope

The provider declares external-process, computer-control, and network
permissions. A provider being installed does not expose its tools to every
conversation; individual tools still need to be attached to a conversation and
approved by Windie's tool policy.

Experimental tools, Chrome extension tools, unrestricted filesystem paths, and
existing-browser attachment are not enabled in this version. The provider does
not use `--autoConnect`, remote-debugging ports, `--browser-url`, or
`--wsEndpoint`.

The user's normal Chrome profile and its open tabs are not used. Windie also
does not currently provide a profile reset/deletion action; remove the profile
manually only when the user explicitly wants to discard its saved sessions.
