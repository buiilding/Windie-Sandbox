# Local components

This overview explains how Windie's independently managed local processes fit
together. Detailed pages cover the [API process](api-process.md),
[gateway process](gateway-process.md), [tray](tray.md), and
[notifier](notifier.md).

## Purpose

Windie runs independent local components for runtime execution, model access,
desktop controls, and notifications. The Inspector is a client of that local
runtime; it does not own its work.

```text
                     [ Inspector ]
                           ↕
[ Bifrost Gateway ]  ↔  [ Windie API ]  →  [ Notifier ]
         ↑                    ↑
         └────── [ Tray ] ────┘
```

The managed local components are the API, Bifrost gateway, tray, and notifier.
Each has its own lifecycle, PID file, and log. The Inspector is a browser
client, not a managed local runtime process.

## API process

The API is Windie's local runtime process. It serves the loopback HTTP and SSE
interfaces and owns session execution, context construction, tool execution,
wakeups, durable events, and SQLite-backed runtime state. `SessionManager` is
part of this process and supervises live session work.

Lifecycle commands: `windie api start`, `windie api stop`, and
`windie api output`.

## Bifrost gateway

The Bifrost gateway is the local OpenAI-compatible connection to configured
LLM providers. It handles provider communication, model discovery, and provider
management. It does not own Windie conversations, sessions, tools, or storage.

Lifecycle commands: `windie gateway start`, `windie gateway stop`, and
`windie gateway output`.

## Inspector

The Inspector is the hosted browser client. It displays Windie state and sends
user actions to the API through the paired local connection. It does not read
SQLite, call Bifrost directly, or own session execution.

In development, `windie dev run inspector` starts a frontend development
server; it is not the local runtime.

## Tray

The tray is an independent desktop component for API and gateway status plus
explicit start and stop controls. It does not supervise the runtime or execute
sessions.

Lifecycle commands: `windie tray start`, `windie tray stop`, and
`windie tray output`.

## Notifier

The notifier observes completed-session events from the API and presents native
completion notifications. It is presentation only: it never runs a model or
changes session state.

Lifecycle commands: `windie notifier start`, `windie notifier stop`, and
`windie notifier output`.

## Main flow

1. The Inspector sends a request to the API.
2. The API resolves the session, builds model context, and sends the request to
   Bifrost.
3. Bifrost calls the configured provider and returns the response to the API.
4. The API saves messages, session state, and events in SQLite.
5. The Inspector receives session updates; the notifier can present a completed
   session as a native notification.

## Component boundaries

- API, Bifrost gateway, tray, and notifier start and stop independently.
- The API owns runtime work and durable Windie state; Bifrost owns provider
  communication.
- The Inspector owns browser presentation and user interaction.
- The tray owns status and explicit controls.
- The notifier owns completion presentation.
- MCP provider processes belong to the tool/MCP layer during execution, not to
  these managed local components.

## Related code

- `src/api/mod.rs` and `src/session/manager.rs` — API startup and session
  supervision.
- `src/store/session.rs` and `src/store/message.rs` — durable runtime state.
- `src/runtime/context.rs` and `src/runtime/turn.rs` — model context and turns.
- `src/llm/gateway.rs` and `src/llm/client.rs` — Bifrost lifecycle and model
  requests.
- `src/local/process.rs`, `src/local/tray.rs`, and `src/local/notifier.rs` —
  local component lifecycle and presentation.
