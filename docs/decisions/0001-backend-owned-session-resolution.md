# Backend-owned session resolution

## Status

Accepted.

## Context

A browser can cache session summaries and display a selected conversation
branch, but it cannot reliably decide which durable session owns a message
head. Another client, a CLI command, a wakeup, or a previous run may have
changed that state after the browser last refreshed.

Guessing in the Inspector would create duplicate sessions, continue the wrong
branch, or hide an ambiguous state behind a convenient UI choice.

## Decision

The API is the runtime authority. A client sends a conversation ID and selected
message head; SQLite resolves whether exactly one session exists at that head,
no session exists, or the result is ambiguous. The API returns that result or
creates the new branch session atomically when the requested operation allows
creation.

The hosted Inspector is a presentation client. It asks the API to query,
continue, approve, cancel, or inspect a session, but it does not run the
model/tool loop or infer durable ownership from its local state.

## Consequences

- Session identity and branch ownership remain consistent across the Inspector,
  CLI, API background tasks, and future clients.
- Closing, refreshing, or disconnecting the Inspector does not stop an
  already-running API-owned session.
- The API can serialize execution with claims and recover durable state after
  a process restart.
- Clients must render the API's resolution result, including ambiguity or a
  stale-head conflict, rather than silently choosing a session.

## Related code

- `src/store/session.rs`: atomic head resolution and session creation.
- `src/operation/session.rs`: shared session lifecycle operations.
- `src/session/manager.rs`: API-owned background execution and claims.
- `src/api/session.rs`: HTTP adapter for session operations.
- `vendor/windie-inspector/frontend/src/hooks/useSessionRuntime.js`: Inspector
  requests and renders backend-owned resolution.
