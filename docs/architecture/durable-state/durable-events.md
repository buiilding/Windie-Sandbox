# Durable events

## Purpose

<!-- Explain what session events record and why they are persisted. -->

## Owns

<!-- Describe event identity, ordering, payloads, replay, and retention. -->

## Does not own

<!-- Distinguish the activity log from conversation messages and session state. -->

## Main flow

1. <!-- Explain when runtime activity creates an event. -->
2. <!-- Explain how the event is committed with related state. -->
3. <!-- Explain how consumers replay and follow events. -->

## Important invariants

- <!-- Events describe durable activity but do not replace canonical records. -->
- <!-- Consumers can resume from a durable cursor without duplicating effects. -->

## Related code

- <!-- `src/session/event.rs` -->
- <!-- `src/store/session.rs` -->
- <!-- `src/api/event.rs` -->
