# Execution claims

## Purpose

<!-- Explain why every session execution attempt needs a unique fencing token. -->

## Owns

<!-- Describe claim acquisition, validation, replacement, and release. -->

## Does not own

<!-- Distinguish execution ownership from session identity and process identity. -->

## Main flow

1. <!-- Show how a runner acquires a claim. -->
2. <!-- Show how writes and terminal transitions validate that claim. -->
3. <!-- Show what happens to stale or duplicate runners. -->

## Important invariants

- <!-- Every execution attempt receives a fresh claim. -->
- <!-- Stale claims cannot append messages, events, or state transitions. -->

## Related code

- <!-- `src/session/id.rs` -->
- <!-- `src/store/session.rs` -->
- <!-- `src/operation/session.rs` -->
