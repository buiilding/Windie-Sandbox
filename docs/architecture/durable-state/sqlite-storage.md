# SQLite storage

## Purpose

<!-- Explain why SQLite is Windie's durable local source of truth. -->

## Owns

<!-- Describe the major persisted records, transactions, indexes, and schema boundary. -->

## Does not own

<!-- Distinguish durable storage from in-memory supervision and model transport. -->

## Main flow

1. <!-- Explain database initialization and schema checks. -->
2. <!-- Explain how runtime operations read and atomically update related records. -->
3. <!-- Explain how later clients reconstruct current state. -->

## Important invariants

- <!-- Record atomicity requirements across messages, session heads, and events. -->
- <!-- Record schema-version and ownership constraints. -->

## Related code

- <!-- `src/store/schema.rs` -->
- <!-- `src/store/message.rs` -->
- <!-- `src/store/session.rs` -->
