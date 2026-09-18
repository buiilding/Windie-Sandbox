# Shared Live Session Event Delivery

## Purpose

Make hosted assistant streaming feel as immediate as the local runtime without
weakening durable event replay, account authorization, or execution-claim
fencing.

This is a focused session-event transport plan. It does not redesign
conversations, session persistence, the Inspector, Bifrost, account-event
SSE, device execution, or the official UI.

## Current behavior

Both runtimes save the same `SessionEvent` / `SessionEventRecord` domain
events. Their live-delivery paths differ:

```text
Local API
session writes SQLite event → SessionManager broadcast → SSE → browser

Hosted API today
hosted worker writes PostgreSQL event → 250 ms PostgreSQL poll → SSE → browser
```

The hosted worker already saves one durable event per model delta. Its
`src/hosted/events.rs` polling interval is therefore the main source of
visible batching. The hosted Inspector appends every received
`assistant_delta`; it cannot display events that have not reached it yet.

The desired common pattern is:

```text
persist committed event → publish the exact record live → browser receives it
reconnect              → replay durable records after its cursor
```

PostgreSQL remains authoritative. A live hub is an optimization for connected
clients, never a second source of truth.

## Design constraints

- Reuse `SessionEvent`, `SessionEventRecord`, and the local subscription
  semantics; do not introduce hosted-specific event payloads.
- A record may be published only after its SQLite or PostgreSQL transaction
  commits successfully.
- SSE retains a durable event-ID cursor. It must suppress duplicate live
  delivery and recover after reconnect, stream lag, process restart, or a
  missed cross-instance notification.
- A hosted SSE endpoint authorizes account ownership before subscribing. A
  live hub must never become an authorization boundary or leak a session event
  across accounts.
- Do not make the browser connect directly to Bifrost or PostgreSQL.
- Do not lower the hosted database polling interval as the primary solution;
  that trades database load for an incomplete approximation of live delivery.
- Do not create a large persistence trait. This is a small transport primitive
  over the existing shared event record.

## Target structure

Extract the local `SessionManager` channel behavior into a small shared
session module, for example:

```text
src/session/live_events.rs
  SessionEventHub
  SessionSubscription

src/session/manager.rs
  local execution and SQLite persistence
  → SessionEventHub::publish(committed record)

src/hosted/runtime.rs
  PostgreSQL persistence and hosted worker
  → SessionEventHub::publish(committed record)

src/api/session.rs
src/hosted/events.rs
  replay durable rows, then consume SessionSubscription
```

`SessionEventHub` is process-local and keyed by `SessionId`. It owns a bounded
`tokio::sync::broadcast` channel per active session. It accepts and yields a
`SessionEventRecord`; it does not know which database wrote the record, which
account owns it, or how to serialize SSE.

Keep lifetime explicit: create a channel when a session first needs live
delivery. The local `SessionManager` may close terminal channels because it
serializes local run handoff. Hosted sessions can be reused by a concurrent
browser after a terminal event, so do not close their hub solely from one
worker; add subscriber-aware idle cleanup before retaining hosted hubs across
long-lived multi-instance production processes.

## SSE handoff invariant

The transition from replay to live delivery must not have an event-loss gap.
For one session endpoint:

```text
1. authenticate and prove account ownership
2. subscribe to the process-local hub
3. load durable events with id > requested cursor
4. send replayed records in durable ID order
5. receive live records; ignore records with id <= current cursor
6. on broadcast lag or closure, replay PostgreSQL/SQLite after current cursor
```

Subscribing before the initial replay means an event committed during replay is
either in the durable replay or waiting in the live subscription. The durable
cursor remains the duplicate-suppression and recovery mechanism.

The existing local endpoint should adopt this same handoff rule while the hub
is extracted. Its current in-process channel is the implementation to reuse,
not behavior to discard.

## Hosted single-instance live path

### Phase 0 — Establish the behavioral baseline

Inspect and preserve:

- `src/session/manager.rs` channel creation, publication, and terminal cleanup;
- `src/api/session.rs` replay and subscription SSE route;
- `src/hosted/runtime.rs` event writer;
- `src/hosted/store.rs` claimed event append transaction;
- `src/hosted/events.rs` durable hosted SSE route; and
- Inspector SSE parsing and cursor behavior.

Add tests that state the delivery contract before moving code:

- a subscriber receives the exact committed event record;
- a subscriber never receives a record from another session;
- replay plus live handoff neither loses nor duplicates an event;
- a lagged/closed subscription recovers from durable records after its cursor;
- an event append rejected by a stale execution claim is not published.

### Phase 1 — Extract the shared in-process hub

Move `SessionSubscription` and the per-session bounded broadcast-channel
ownership out of `SessionManager` into `src/session/live_events.rs`. Keep its
public behavior narrow: subscribe, publish a committed record, and release a
terminal session channel.

Rewire the local `SessionManager` to use the shared hub without changing its
SQLite event order, local SSE wire format, or tool/runtime behavior. Run the
existing local session/API tests as the compatibility oracle.

### Phase 2 — Publish committed hosted records immediately

Add one `SessionEventHub` to hosted server state and pass it to
`HostedRuntime` and the hosted session SSE route.

When `HostedStore::append_session_execution_event` returns a committed
`SessionEventRecord`, `HostedRuntime` publishes that returned record to the
hub. The same rule applies to terminal records returned by completion,
cancellation, failure, queued-input materialization, and wakeup workflows.
No code publishes an event before its transaction commits.

Change hosted session SSE from normal 250 ms polling to replay-plus-live-hub
delivery. Retain PostgreSQL replay on initial connection, after a lagged hub
receiver, and after a stream reconnect. Keep account ownership checks before
subscription and on durable reads.

Acceptance criteria:

- a live hosted subscriber receives a persisted assistant delta without
  waiting for `EVENT_POLL_INTERVAL`;
- refresh/reconnect replays only later durable records and never reruns a
  model request;
- a server restart still recovers all durable session state/events;
- existing local SSE tests retain their behavior.

## Multiple hosted-server instances

The process-local hub only reaches browsers attached to the same
`windie-server` process. Before horizontally scaling the hosted service, add a
PostgreSQL notification bridge.

### Phase 3 — Durable cross-instance wakeups

In the transaction that inserts a session event, issue a PostgreSQL `NOTIFY`
for a fixed internal channel with the committed event ID as its small payload.
PostgreSQL delivers `NOTIFY` only after commit, so it preserves the
commit-before-publish invariant.

Each `windie-server` process owns one `PgListener` task. On a notification it:

1. loads the referenced durable record from PostgreSQL;
2. publishes it through that process's `SessionEventHub`; and
3. lets its already-authorized SSE subscribers filter it by their durable
   cursor.

The originating process may publish directly after commit for minimum latency;
its own notification is harmless because the SSE cursor suppresses the
duplicate. Notifications are wakeups, not data authority: if one is lost,
late, duplicated, or its listener restarts, durable replay after the cursor
repairs the stream.

Do not put event bodies, tokens, account identifiers, credentials, or model
content in the notification payload. The listener reloads the record from the
private database.

Acceptance criteria:

- two independently constructed hosted server states connected to the isolated
  PostgreSQL test database deliver a committed record to subscribers of both;
- a listener restart or intentionally missed notification is repaired by
  durable replay;
- no subscriber receives an event for an unauthorized account/session;
- duplicate notifications do not produce duplicate SSE event IDs.

## Inspector scope

The existing hosted Inspector bridge already incrementally appends
`assistant_delta` records. It should need no protocol change for this plan.
After transport verification, a small `requestAnimationFrame` paint buffer may
be considered only if frequent React renders become visually expensive. That
is a presentation optimization, not a replacement for immediate server
delivery and must not coalesce or lose durable event data.

## Documentation and verification

Update the following when implementation begins:

- `docs/index/Backend.md` for the extracted shared event hub and hosted
  replay/live behavior;
- `docs/plans/main-hosted-windie-server.md` with the Phase 7 streaming status;
- `memory/hosted-windie-server-context.md` with the deployment/runtime
  checkpoint.

Required verification:

```text
cargo fmt --check
cargo test session --lib
cargo test api --lib
cargo test hosted --lib
isolated PostgreSQL hosted acceptance tests
Inspector unit tests and production build if frontend code changes
```

Finally, Peter manually verifies a fresh hosted conversation at
`app.windieos.com`: the first assistant text arrives smoothly, a refresh
recovers its final durable message, and a second signed-in browser receives
the same result. Do not use browser automation for that authenticated proof
unless Peter explicitly asks.
