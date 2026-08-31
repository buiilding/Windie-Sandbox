# Server-Sent Events

## Purpose

<!-- Explain how clients replay and follow session activity over HTTP. -->

## Owns

<!-- Describe stream endpoints, cursors, event serialization, replay, and live delivery. -->

## Does not own

<!-- Distinguish SSE transport from the durable event store and session truth. -->

## Main flow

1. <!-- Open a session-specific or aggregate stream with a cursor. -->
2. <!-- Replay persisted events after that cursor. -->
3. <!-- Continue delivering live events and reconnect safely. -->

## Important invariants

- <!-- Consumers tolerate reconnects and suppress duplicate event IDs. -->
- <!-- Hydrated presentation data remains derived from authoritative records. -->

## Related code

- <!-- `src/api/sse.rs` -->
- <!-- `src/api/event.rs` -->
- <!-- Inspector session-stream modules. -->
