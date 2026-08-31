# API process

## Purpose

<!-- Explain the API as the local process that hosts Windie's runtime authority. -->

## Owns

<!-- Describe startup, shared state, SessionManager, schedulers, HTTP, and shutdown. -->

## Does not own

<!-- Distinguish the API process from Bifrost and independently managed peers. -->

## Main flow

1. <!-- Initialize durable state and recover interrupted sessions safely. -->
2. <!-- Start HTTP, SSE, session supervision, and wakeup scheduling. -->
3. <!-- Drain or stop owned work during graceful shutdown. -->

## Important invariants

- <!-- SessionManager is an in-process component, not another service. -->
- <!-- Restart never blindly replays an interrupted external request. -->

## Related code

- <!-- `src/api/mod.rs` -->
- <!-- `src/api/state.rs` -->
- <!-- `src/session/manager.rs` -->
