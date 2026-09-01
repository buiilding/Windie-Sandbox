# Session lifecycle

## Purpose

<!-- Explain how a durable session moves between lifecycle states. -->

## Owns

<!-- Describe the states, transitions, queued work, and restart behavior. -->

## Does not own

<!-- Distinguish the session record from conversation messages and live runtime work. -->

## Main flow

1. <!-- Explain how a session is resolved or created. -->
2. <!-- Follow it through running, approval, completion, failure, or cancellation. -->
3. <!-- Explain how later input or a wakeup can resume durable work. -->

## Important invariants

- <!-- Record which transitions are allowed and who may perform them. -->
- <!-- Record what survives an API restart and what cannot be replayed safely. -->

## Related code

- <!-- `src/session/model.rs` -->
- <!-- `src/store/session.rs` -->
- <!-- `src/session/manager.rs` -->
