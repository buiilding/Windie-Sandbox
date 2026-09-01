# Interfaces

This overview explains how clients communicate with Windie. Detailed pages
cover the [HTTP API](api.md), [Server-Sent Events](server-sent-events.md),
[CLI](cli.md), and [Inspector](inspector.md).

## Purpose

The Windie API is the local runtime boundary. The Inspector is the hosted
browser client that lets a paired user inspect that runtime and request
actions. The CLI is another client of shared runtime operations, while SSE is
the streaming boundary used to replay and follow session activity. Keeping
these responsibilities separate lets API-owned session work continue when the
browser tab closes, reloads, or disconnects.

```text
Hosted Inspector in the browser
              │ HTTP and SSE
              v
Local Windie API on the user's computer
              │
              ├── SQLite conversations, sessions, and events
              ├── SessionManager and runtime turns
              ├── tool approval and tool execution
              └── Bifrost gateway for model requests
```

## Interface roles

- The **API** exposes authoritative runtime operations over loopback HTTP.
- **SSE** replays and streams durable session activity to connected consumers.
- The **CLI** parses terminal commands and invokes the same shared operations
  used behind the API.
- The **Inspector** is the hosted visual client for inspecting state and
  sending user actions to a paired local API.

## API owns

The API owns authoritative runtime work:

- resolving or creating the session for a requested conversation head;
- starting and supervising in-process session work, wakeups, approval, and
  cancellation;
- compiling model context and routing model requests through Bifrost;
- saving messages, session state, and durable execution events in SQLite; and
- replaying and streaming session updates over Server-Sent Events (SSE).

The API is loopback-bound. Its normal client is the paired hosted Inspector,
and the API checks that client access before serving runtime data.

## Inspector owns

The Inspector owns browser presentation and interaction:

- selecting conversations, branches, and views;
- rendering the tree, selected-path transcript, execution progress, and
  approval controls;
- sending explicit user actions to the API; and
- keeping short-lived browser state such as in-progress visual updates.

The Inspector does not read SQLite, contact Bifrost directly, execute tools,
or decide what the model sees. It receives the API's resolved session rather
than deriving one from its cached session list.

## Main flow

1. The Inspector asks the API to query or continue a conversation at a chosen
   message head.
2. The API resolves the durable branch session in SQLite. It returns an
   existing matching session, creates one if no session matches, or rejects an
   ambiguous or stale request.
3. The API's session manager runs the work. The runtime persists its progress
   and emits durable events as assistant messages, tool results, approvals, or
   lifecycle state change.
4. The Inspector receives replayed and live SSE events, then refreshes its
   presentation from authoritative snapshots when necessary.

Closing the Inspector does not stop a running session. Conversely, an API
restart cannot continue an interrupted provider or tool request automatically:
the API records that attempt as failed to avoid duplicating external work.

## Related references

- [Sessions](../durable-state/sessions.md) — branch resolution, execution
  claims, and recovery.
- [Runtime loop](../execution/runtime-loop.md) — the work the API runs for a
  session.
- [Local components](../local-components/) — API, Bifrost, tray, and notifier
  lifecycle.
- [Backend-owned session resolution](../../decisions/0001-backend-owned-session-resolution.md)
  — the decision behind API-owned session resolution.
