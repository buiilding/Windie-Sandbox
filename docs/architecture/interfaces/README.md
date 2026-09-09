# Interfaces

This overview explains how clients communicate with Windie. Detailed pages
cover the [HTTP API](api.md), including its Server-Sent Events (SSE) streams,
the [CLI](cli.md), and the [Inspector](inspector.md).

## Purpose

Windie has two client interfaces and one streaming transport. The API is the
local runtime boundary, the Inspector is the browser client, and the CLI is
the terminal client. Server-Sent Events (SSE) is the streaming boundary used
to replay and follow durable session activity.

Keeping these responsibilities separate lets the API own runtime work while
the Inspector and CLI remain clients of shared operations and persisted state.

```text
Inspector (browser) ── HTTP requests ──┐
Inspector (browser) <── SSE events ────┤
                                      v
CLI (terminal) ─── shared operations ──> Windie runtime
                                      │
                                      ├── SQLite state and durable events
                                      └── Bifrost model requests
```

## Interface roles

- The **API** exposes authoritative runtime operations over loopback HTTP.
- **SSE** replays and streams durable session activity to connected consumers.
- The **CLI** parses terminal commands and invokes the same shared operations
  used behind the API.
- The **Inspector** is the hosted visual client for inspecting state and
  sending user actions to a paired local API.

## Main flow

1. The Inspector sends an HTTP request, or the CLI invokes a terminal adapter,
   with an explicit conversation, message head, or session target.
2. The API route or CLI adapter delegates to the shared operation layer. The
   API resolves the durable branch session in SQLite; a CLI session claims the
   same execution model with the CLI owner.
3. The runtime builds context, routes model requests through Bifrost, applies
   tool approval, executes allowed tools, and persists messages, session state,
   and durable events.
4. The API returns an HTTP response and emits durable session events. The
   Inspector or another SSE consumer replays events after its cursor and then
   follows new events as they arrive.

Closing the Inspector does not stop a running session. Conversely, an API
restart cannot continue an interrupted provider or tool request automatically:
the API records that attempt as failed to avoid duplicating external work.

## Interface boundaries

- The **API** owns authoritative runtime work: session resolution and
  supervision, context construction, tool policy and execution, persistence,
  and the HTTP/SSE transport. It is loopback-bound and checks client access
  before serving protected runtime data.
- **SSE** owns transport for replaying and following durable session events. It
  does not replace SQLite or become the source of conversation truth.
- The **CLI** owns argument parsing, terminal adapters, and output formatting.
  It calls the same shared operations and persistence rules as the API; output
  formatting does not make runtime decisions.
- The **Inspector** owns browser routes, conversation-tree presentation,
  controls, and short-lived streaming state. It does not read SQLite, contact
  Bifrost directly, execute tools, or infer session ownership from cached
  browser state.

## References

- [Sessions](../storage/sessions.md) — branch resolution, execution
  claims, and recovery.
- [Runtime loop](../execution/runtime-loop.md) — the work the API runs for a
  session.
- [Components](../components/) — API, Bifrost, tray, and notifier
  lifecycle.
- [Backend-owned session resolution](../../decisions/0001-backend-owned-session-resolution.md)
  — the decision behind API-owned session resolution.
