# Notifier

## Purpose

<!-- Explain the independent process that presents native completion notifications. -->

## Owns

<!-- Describe aggregate event observation, cursor persistence, previews, and click actions. -->

## Does not own

<!-- Distinguish notification presentation from session execution and durable truth. -->

## Main flow

1. <!-- Connect to the aggregate completion event stream. -->
2. <!-- Resume from the persisted cursor and select canonical completions. -->
3. <!-- Present the notification and record the consumed cursor. -->

## Important invariants

- <!-- Notification delivery never changes session state. -->
- <!-- Reconnect behavior avoids presenting the same durable completion twice. -->

## Related code

- <!-- `src/local/notifier.rs` -->
- <!-- `src/local/session_event_observer.rs` -->
- <!-- `src/local/tray_notification.rs` -->
