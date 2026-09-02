# API

The API is the local server that hosts Windie's runtime primitives. It is a
running process that listens for client requests and transmits results and
live session events back to those clients. By default, it listens on
`http://127.0.0.1:8787`.

The client-facing route and authorization contract is documented in the
[HTTP API reference](../interfaces/api.md).

## Owns

The API process owns the runtime boundary between clients and Windie's core
systems. It:

- initializes the SQLite-backed store, plugin catalog, tool registry, and
  session manager;
- supervises durable sessions, wakeups, approvals, cancellation, and recovery;
- builds model context and routes model requests through the Bifrost gateway;
- executes approved tools and persists messages, session state, and events;
- serves the HTTP and Server-Sent Events interfaces; and
- handles graceful process shutdown.

## Does not own

The API does not perform provider inference itself. Bifrost is the separate
gateway responsible for communicating with configured LLM providers. The API
also does not own browser presentation: the Inspector displays API responses
and sends user actions, while the tray and notifier are independent local
components.

The API process is not the same thing as a Windie conversation or session. It
supervises those durable records and runtime tasks, but the records remain
stored in SQLite and can outlive a browser tab or one API process lifetime.

## Main flow

1. The API process initializes its SQLite-backed state, plugin catalog, tool
   registry, and session manager.
2. It starts the idle-wakeup scheduler and recovery for interrupted durable
   sessions, then binds the loopback HTTP listener.
3. A client request enters through the HTTP interface. The route handler
   delegates to the shared conversation, session, tool, provider, or component
   operation instead of implementing a second runtime path.
4. The API's session manager runs requested work. The runtime builds context,
   sends model requests to Bifrost, executes approved tools, and persists
   messages and durable events.
5. The API returns a response or streams replayable and live session events.
   When shutdown is requested, it signals the server and stops its supervised
   work before the process exits.

## Important invariants

- The API is bound to the local machine. Its HTTP access and authorization
  rules are defined by the [HTTP API contract](../interfaces/api.md).
- The API is the authority for session ownership and conversation-head
  resolution. The Inspector does not infer either from cached browser state.
- The API owns durable runtime state; the Inspector owns presentation state.
- Bifrost remains a separate provider boundary and does not own Windie
  conversations, sessions, tools, or SQLite state.
- Session work continues in the API process even if an Inspector tab closes or
  loses its SSE connection.
- API restart recovery does not blindly replay an interrupted external model
  or tool request.

## Related code

- [`src/api/mod.rs`](../../../src/api/mod.rs) — starts the API process and
  initializes shared runtime state.
- [`src/api/state.rs`](../../../src/api/state.rs) — state shared by route
  handlers.
- [`src/session/manager.rs`](../../../src/session/manager.rs) — supervises
  durable session execution inside the API process.
- [`src/operation/`](../../../src/operation/) — shared workflows used by the
  API and CLI adapters.
- [HTTP API](../interfaces/api.md) — client-facing routes and contracts.
