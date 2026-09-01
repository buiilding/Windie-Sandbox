# Wakeups

## Purpose

<!-- Explain the common activation primitive for user and automatic work. -->

## Owns

<!-- Describe wakeup kinds, provenance, scheduling, and transcript behavior. -->

## Does not own

<!-- Distinguish activation from session lifecycle and runtime execution. -->

## Main flow

1. <!-- Explain where a wakeup originates. -->
2. <!-- Explain how it targets and activates a session. -->
3. <!-- Explain how the runtime receives durable wakeup input. -->

## Important invariants

- <!-- Wakeups use the same execution path as normal user input. -->
- <!-- Provenance and permission boundaries remain inspectable. -->

## Related code

- <!-- `src/runtime/wakeup.rs` -->
- <!-- `src/session/manager.rs` -->
