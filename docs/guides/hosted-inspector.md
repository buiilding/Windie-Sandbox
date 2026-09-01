# Inspector access

## Purpose

Windie has two Inspector entry points for the same local runtime:

- `windie inspector open` opens the packaged frontend served from the local
  API. It needs no hosted account.
- `https://app.windieos.com` serves the hosted frontend. It remains available
  for a user who wants the web deployment and signs in with a Windie account.

Neither is a hosted runtime or a proxy to a remote API. Both browser clients
talk directly to the local API at `http://127.0.0.1:8787` by default. The
browser makes that loopback connection; `app.windieos.com` never receives the
conversation, tool call, or provider request as a runtime relay.

## Local connection flow

```text
windie inspector open
             │
             v
CLI proves possession of Windie's private component credential to the API
             │
             v
API returns one one-time, 60-second launch code
             │
             v
browser receives it only in a URL fragment and exchanges it once
             │
             v
browser uses an API-restart-scoped local token for requests and SSE
```

The long-lived component credential is never exposed to the browser. Closing
the tab drops its `sessionStorage` token; restarting the API invalidates all
local browser tokens. An arbitrary page on loopback cannot create one because
only a Windie peer process can mint the one-time code.

## Hosted connection flow

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

The API allows CORS requests from the exact hosted Inspector origin and local
development origins. It accepts a hosted bearer token only for a loopback API
URL, then checks that the account has been explicitly paired with that local
runtime. A different account cannot silently take over a paired runtime.

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
also permits through CORS. Once the development server is healthy, Windie opens
it with the same local one-time-code exchange as a packaged Inspector. API URL
overrides are available through `window.__WINDIE_API_URL__` or
`REACT_APP_WINDIE_API_URL`; production defaults to the loopback API address.

## Related code

- `src/api/router.rs`: allowed browser origins and API routes.
- `src/api/runtime_access.rs`: hosted-account validation, local launch-code
  exchange, and pairing middleware.
- `src/inspector.rs`: local launch-code request, OS browser opening, and
  packaged-asset discovery.
- `src/config.rs`: loopback API default.
- `vendor/windie-inspector/frontend/src/lib/windieApi.js`: browser HTTP and
  authorization boundary.
- `vendor/windie-inspector/frontend/src/hooks/useSessionRuntime.js`: session
  selection, API operations, and SSE-backed presentation.
