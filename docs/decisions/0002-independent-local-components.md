# Independent local components

## Status

Accepted.

## Context

Windie needs model access, durable runtime execution, desktop controls, and
notifications. Treating those responsibilities as one supervising desktop
application would make a UI or notification failure capable of interrupting
runtime work, and would make each component harder to restart, inspect, and
replace.

## Decision

Run the Bifrost gateway, Windie API, tray, and notifier as independent local
components with their own process lifecycle, PID file, and log. The hosted
Inspector is a browser client of the API, not a managed local runtime process.

The API owns durable runtime work. Bifrost owns provider communication. The
tray owns explicit status and start/stop controls. The notifier owns native
completion presentation.

## Consequences

- Stopping the Inspector, tray, or notifier does not stop API session work.
- The API and Bifrost gateway can be restarted independently, and a notifier
  can reconnect to durable completion events after a temporary failure.
- The tray is a control surface, not a supervisor for the API, gateway, or
  notifier.
- Components communicate through explicit local APIs and durable state rather
  than hidden in-process callbacks.
- Cross-process coordination must tolerate component absence and restart; it
  cannot assume every presentation component is currently running.

## Related code

- `src/local/process.rs`: managed component PID, log, start, and stop support.
- `src/api/mod.rs`: API-owned session manager and wakeup scheduler.
- `src/llm/gateway.rs`: Bifrost gateway lifecycle.
- `src/local/tray.rs`: tray status and explicit controls.
- `src/local/notifier.rs`: independent notifier process.
