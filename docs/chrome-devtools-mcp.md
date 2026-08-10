# Chrome DevTools MCP

Windie provides the official Chrome DevTools MCP as the `chrome-devtools`
provider.

## Connection modes

Windie offers two explicit connection modes when Chrome DevTools is installed:

- **Windie-managed Chrome** launches the persistent profile described below.
  This is the default and keeps Windie's browser state separate from the
  user's normal Chrome.
- **Existing Chrome** attaches to the user's already-running Chrome. Chrome
  144 or newer must be running with remote debugging enabled from
  `chrome://inspect/#remote-debugging`. Windie first checks TCP reachability at
  `127.0.0.1:9222`. If the port is not listening, the Inspector opens Chrome's
  settings page and waits until the user enables the setting. Windie then
  launches the MCP with `--auto-connect`.

The existing-Chrome flow does not pass Windie's `--user-data-dir`. The TCP
check only confirms that Chrome is listening; MCP still handles its own
approval and readiness handshake.

## Managed Chrome behavior

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

## Switching modes

The provider's **Configure** action switches between the two modes. Windie
stops the current MCP session, updates the saved mode, starts MCP with the new
arguments, and rediscovers the catalog and readiness state. The npm package
and managed runtime are reused; they are not downloaded again.

Stopping an existing-Chrome MCP session only disconnects Windie's MCP process;
it does not terminate the user's Chrome process.

## Safety and scope

The provider declares external-process, computer-control, and network
permissions. A provider being installed does not expose its tools to every
conversation; individual tools still need to be attached to a conversation and
approved by Windie's tool policy.

Experimental tools, Chrome extension tools, unrestricted filesystem paths, and
manual `--browser-url`/`--wsEndpoint` attachment are not enabled. Existing
Chrome attachment is available only through Chrome's `--auto-connect` approval
flow.

The user's normal Chrome profile and its open tabs are not used. Windie also
does not currently provide a profile reset/deletion action; remove the profile
manually only when the user explicitly wants to discard its saved sessions.
