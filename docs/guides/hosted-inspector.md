# Hosted Inspector

## Purpose

`https://app.windieos.com` serves the Windie Inspector: a hosted browser client
for a Windie runtime running on the same computer. It is not a hosted runtime
or a proxy to a remote API.

When the page runs in a user's browser, it talks directly to the local API at
`http://127.0.0.1:8787` by default. The browser makes that loopback connection;
`app.windieos.com` never receives the conversation, tool call, or provider
request as a runtime relay.

## Connection flow

```text
browser loads app.windieos.com
             │
             v
browser signs in and obtains a hosted account token
             │
             v
browser calls the local API at 127.0.0.1:8787 with that token
             │
             v
local API validates the account and explicit local-runtime pairing
             │
             v
Inspector reads snapshots, sends actions, and subscribes to session SSE
```

The API allows CORS requests from the exact hosted Inspector origin and the
local development origins. It accepts the Inspector's bearer token only for a
loopback API URL, then checks that the account has been explicitly paired with
that local runtime. A different account cannot silently take over a paired
runtime.

## Runtime boundary

The Inspector owns browser presentation: selected conversation, selected
session, forms, temporary streaming previews, and rendering. It does not read
SQLite, call Bifrost, execute tools, or decide which session owns a branch.

When the user sends input, the Inspector asks the local API to resolve or
create the session and queue/run it. The API owns the background session task.
Closing the browser tab, refreshing the page, or losing the session SSE stream
does not cancel that task. On return, the Inspector reloads authoritative
snapshots and can replay session events after its cursor.

The addressable route `/sessions/<session-id>` is also presentation-only: it
asks the local API for that durable session before selecting its conversation.

## Development

`windie dev run inspector` starts a local frontend development server. It is a
developer convenience, not the runtime, and uses a local origin that the API
also permits through CORS. API URL overrides are available through
`window.__WINDIE_API_URL__` or `REACT_APP_WINDIE_API_URL`; production defaults
to the loopback API address.

## Related code

- `src/api/router.rs`: allowed browser origins and API routes.
- `src/api/runtime_access.rs`: hosted-account validation and local pairing.
- `src/config.rs`: loopback API default.
- `vendor/windie-inspector/frontend/src/lib/windieApi.js`: browser HTTP and
  authorization boundary.
- `vendor/windie-inspector/frontend/src/hooks/useSessionRuntime.js`: session
  selection, API operations, and SSE-backed presentation.
