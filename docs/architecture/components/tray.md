# Tray

## Purpose

The tray is an independent desktop control for the Windie API and the Bifrost
LLM gateway. It shows whether those two local processes are running and lets
the user request that either process start or stop.

The tray is currently supported on macOS and Windows. It is a control surface,
not the parent process or supervisor for the runtime.

## Owns

- Polling the API health endpoint and Bifrost health endpoint every 500
  milliseconds to update the menu labels.
- The `Start/Stop Gateway`, `Start/Stop API`, and `Quit Tray` menu actions.
- Sending one explicit start or stop request at a time through Windie's shared
  component lifecycle operations.
- Keeping its own tray process registration and cleaning up its tray PID record
  when the tray exits.

## Does not own

- API sessions, conversations, tools, or other durable runtime state.
- The API or gateway process lifetime after the tray exits.
- Notification delivery or the notifier process.
- The CLI itself. The tray uses the same operation layer as the CLI, but it
  does not invoke a CLI command to perform each action.

## Main flow

1. The tray starts independently and begins polling the API and gateway health
   state.
2. It displays `Start` or `Stop` for each component based on the latest health
   result.
3. When the user selects an action, the tray sends the request in a worker so
   the desktop menu remains responsive.
4. Starting the API uses the shared API lifecycle operation, which launches a
   detached `windie api run` process. Starting the gateway uses the shared
   gateway lifecycle operation, which launches the Windie-owned Bifrost binary.
5. Stopping the API first sends `POST /api/shutdown` and waits for the process
   to exit. If that graceful request is unavailable, the lifecycle boundary
   falls back to stopping the verified recorded process.
6. Stopping the gateway verifies the Bifrost process identity and terminates
   that process; it does not use an API shutdown request.
7. Selecting `Quit Tray` exits only the tray event loop. It does not stop the
   API or gateway.

## Important invariants

- The tray does not become the parent supervisor of local components.
- Closing the tray does not stop API-owned session work, the API process, or
  the Bifrost gateway.
- Start and stop actions affect only the component selected in the menu.
- The tray observes health and requests lifecycle changes; it does not own
  provider communication, session execution, or durable state.
- The gateway and API can be started or stopped independently through their CLI
  commands even when the tray is not running.

## Related code

- [`src/local/tray.rs`](../../../src/local/tray.rs) — tray menu, health polling,
  and user-requested component actions.
- [`src/operation/system.rs`](../../../src/operation/system.rs) — shared
  lifecycle operations used by both the tray and CLI.
- [`src/local/process.rs`](../../../src/local/process.rs) — detached process
  start/stop behavior, PID records, and graceful API shutdown fallback.
- [`src/llm/gateway.rs`](../../../src/llm/gateway.rs) — Bifrost health checks
  and verified gateway process termination.
