# Shared operation and persistence refactor plan

## Implementation status — 2026-09-17

The initial conversation-tree slice is implemented:

- `src/conversation/tree.rs` owns validated parent graphs, selected paths,
  ordinary splice-delete plans, and truncation plans.
- `src/store/message.rs` and `src/hosted/store.rs` both invoke that shared
  policy from their own SQLite/PostgreSQL transaction adapters.
- Fork path selection now uses the same policy in both backends.
- The temporary `src/operation/hosted_conversation.rs` was removed. Its thin
  hosted HTTP adapter now lives at `src/hosted/conversation.rs`.
- SQLite-specific tool-call group selection, session repair, compaction repair,
  and local runtime behavior remain deliberately local persistence concerns.
- Hosted account ownership, revision checks, idempotency, and durable events
  remain deliberately PostgreSQL concerns.

The initial hosted-session slice is now implemented without adding
`hosted_session.rs`:

- `src/session/policy.rs` owns storage-independent session-head resolution and
  execution eligibility. SQLite uses the shared resolution policy; PostgreSQL
  uses both policies while retaining its own locked transactions.
- `src/hosted/store.rs` applies account-scoped PostgreSQL session, claim,
  queue, event, and wakeup writes.
- `src/hosted/runtime.rs` is hosted execution orchestration, not a duplicate
  operation layer. It uses shared session types, a private Bifrost client, and
  no local tool executor.

The device-agent tool workflow remains later work: it must add an explicit
dispatcher/executor contract, rather than turning the hosted worker into an
MCP or filesystem executor.

## Goal

Make local SQLite and hosted PostgreSQL implement one set of Windie
conversation and session rules without creating parallel `operation/` modules
such as `hosted_conversation.rs` and `hosted_session.rs`.

The destination is one canonical operation policy for each domain concern and
two persistence implementations that apply its result atomically:

```text
conversation/tree.rs
  shared parent-tree validation and planned changes

operation/
  client-facing workflows that compose canonical domain policy

src/store/
  SQLite loading and atomic application of planned changes

src/hosted/store.rs
  PostgreSQL loading and atomic application of the same planned changes
  + account ownership + revision guard + idempotency + durable event
```

This preserves Windie's existing conversation-tree semantics. It does not
create a second cloud chat model.

## Current state

The current local operation functions are mostly typed directly against the
synchronous SQLite `Store`. The hosted server initially added
`src/operation/hosted_conversation.rs` so HTTP handlers would not directly
orchestrate PostgreSQL mutations.

That file is an acceptable short-term boundary improvement, but it must not
become a duplicate hosted equivalent for every local operation. Its tree rules
need to migrate into the canonical shared conversation operation as this plan
is executed.

## Non-goals

- Do not replace SQLite with PostgreSQL for the local runtime.
- Do not make `Store` and `HostedStore` implement one massive trait for all
  conversations, sessions, tools, plugins, gateway configuration, and future
  device execution.
- Do not add hosted tool execution, registered devices, VMs, or remote control.
- Do not alter the canonical tree model, selected-head semantics, or the rule
  that the backend resolves durable session ownership.
- Do not require the local runtime to adopt hosted accounts, idempotency keys,
  or cloud revision numbers merely to share tree behavior.

## Target model

### 1. Shared domain commands and planned changes

Put pure reusable graph meaning in the canonical conversation domain module,
`src/conversation/tree.rs`, so both persistence backends can depend on it
without making `store/` depend upward on `operation/`. Keep client-facing
workflow composition in `src/operation/conversation.rs` and related message
operations, not a hosted-named peer module.

The shared layer should express types equivalent to:

```rust
ConversationCommand
ConversationSnapshot
PlannedConversationChange
ConversationPolicyError
```

`ConversationSnapshot` is the in-memory canonical tree and its required
conversation metadata. `ConversationCommand` is an explicit intent such as
append, update, remove-and-splice, truncate, or fork. A pure policy function
validates a command against a snapshot and returns a
`PlannedConversationChange`.

The planned change describes semantic work, not SQL:

```text
Remove message M
  -> confirm M belongs to this conversation
  -> determine M's parent
  -> reconnect M's children to that parent
  -> remove M and its parts
```

It must not contain SQLite SQL, PostgreSQL SQL, an account ID from a browser,
an HTTP response, or a live event transport decision.

### 2. Backend-owned atomic application

SQLite and PostgreSQL each load the necessary snapshot and apply the planned
change inside their own transaction.

```text
SQLite mutation
  begin SQLite transaction
  -> load canonical conversation snapshot
  -> run shared policy
  -> apply resulting patch
  -> commit

Hosted mutation
  begin PostgreSQL transaction
  -> prove account owns the conversation
  -> enforce expected revision / lock the current state
  -> load canonical conversation snapshot
  -> run the same shared policy
  -> apply resulting patch
  -> increment revision
  -> save idempotent response
  -> append durable account event
  -> commit
```

The policy is shared; the transaction mechanism is deliberately not.

### 3. Hosted concerns remain hosted

The PostgreSQL path owns the requirements that only exist because it is a
multi-account, concurrent network service:

- account filtering and authorization;
- expected-revision conflict detection;
- idempotency records and replayed responses;
- durable event records and replay cursors;
- server-safe concurrent transaction behavior.

These are persistence/transport concerns around a successful semantic change.
They should not leak into the local tree policy.

### 4. No universal database trait initially

Do not start by defining an asynchronous trait that tries to model every query
and transaction for SQLite and PostgreSQL. The two backends have different
execution models, and forcing the local synchronous runtime through a broad
async abstraction would add complexity without proving reuse.

Instead, share concrete command, snapshot, and patch types plus pure policy
functions. Each storage implementation invokes that policy from its own small
transaction boundary. Introduce a narrow per-domain persistence port only if
two callers genuinely need one after the policy extraction is stable.

## Migration phases

### Phase 0 — Establish behavioral oracles

Before moving code, inventory the current behavior and tests in:

- `src/api/conversation.rs` and `src/api/message.rs`;
- `src/operation/conversation.rs` and `src/operation/message.rs`;
- `src/store/conversation.rs` and `src/store/message.rs`;
- `src/store/tests.rs`, `src/operation/tests.rs`, and API tests;
- `src/hosted/store.rs` and hosted tests.

Create or preserve tests for these observable rules:

- message parent ownership and valid parent linkage;
- root-to-selected-head path construction;
- remove-and-splice behavior;
- truncation behavior;
- forked conversation content and selected path;
- message parts and role validation;
- failure behavior for unknown or cross-conversation message IDs.

The local behavior is the starting oracle unless a documented hosted safety
requirement intentionally differs.

### Phase 1 — Extract read-only canonical tree projection

Extract shared snapshot-building and tree-validation code first. Do not change
database writes yet.

Both persistence backends must be able to map their rows into the same
conversation/message/part domain representation. This exposes genuine schema
differences early and gives policy functions one input shape.

Acceptance criteria:

- SQLite and PostgreSQL fixtures produce equivalent canonical snapshots for
  the same tree.
- Selected-head path tests pass for both fixtures.
- No API response contract changes merely because the code moved.

### Phase 2 — Extract one mutation policy at a time

Move rules in this order:

1. append message;
2. update message content and parts;
3. remove message with child splicing;
4. truncate descendants after a message;
5. fork at a selected head.

For each operation:

1. Define the typed shared command.
2. Define or reuse the canonical snapshot representation.
3. Implement a pure policy that returns a planned change or typed domain
   error.
4. Make SQLite apply that plan in its current transaction.
5. Make PostgreSQL apply the exact plan in its account-scoped transaction.
6. Run the same behavior tests against both implementations.

Do not migrate all tree operations in one large patch.

### Phase 3 — Rewire local operations and hosted API

Keep client adapters thin:

```text
local API / CLI
  -> canonical operation command
  -> SQLite transaction adapter

hosted API
  -> authenticated account + canonical operation command
  -> PostgreSQL transaction adapter
```

The hosted API remains responsible for HTTP parsing, bearer authentication,
and HTTP error mapping. It must not encode tree policy or issue persistence
steps itself.

The local API remains responsible for local authorization and local HTTP error
mapping. It must not gain hosted account assumptions.

### Phase 4 — Retire the hosted operation duplicate

After all currently supported hosted conversation mutations use canonical
commands and shared policy functions:

- remove `src/operation/hosted_conversation.rs`, or reduce it to an
  unambiguous hosted transport adapter with no domain rules;
- remove hosted-only command copies that duplicate canonical commands;
- update `docs/index/Backend.md` to describe the final boundary;
- ensure no handler calls raw conversation mutation SQL orchestration.

The preferred final state has no `hosted_conversation.rs` at all.

### Phase 5 — Apply the pattern to sessions only when hosted sessions begin

Do not create `hosted_session.rs` in advance. When Phase 7 of the hosted
server plan begins:

1. inventory shared session rules in `src/operation/session.rs`,
   `src/store/session.rs`, and the runtime;
2. extract pure branch-resolution, queue, claim, and lifecycle policy where
   it is currently mixed with SQLite writes;
3. keep local and hosted session persistence adapters separate;
4. preserve the backend-owned session-head resolution rule;
5. separately design the later tool-execution dispatcher/device protocol.

Hosted session persistence will add claims, durable events, wakeups, and
account scoping. Those additions must not cause a parallel session-policy
module.

## Conformance testing

Each extracted operation requires backend-neutral scenario tests. A shared
scenario should run against a SQLite fixture and an isolated PostgreSQL
fixture, then compare observable domain results rather than SQL rows.

Examples:

```text
same tree + remove middle node
  -> same resulting parent/child graph in SQLite and PostgreSQL

same tree + truncate after selected node
  -> same surviving graph and selected-path behavior

same tree + fork at selected head
  -> same copied root-to-head path
```

Hosted-only tests remain separate for account isolation, stale revision
conflicts, idempotent retries, event replay, and restart recovery. SQLite-only
tests remain separate for local runtime access and local process behavior.

The isolated PostgreSQL test database is mandatory for PostgreSQL conformance
tests. It must never point at the production hosted database.

## Completion criteria

This refactor is complete when:

- conversation tree behavior is defined once in canonical operation policy;
- SQLite and PostgreSQL both execute the resulting planned change atomically;
- no duplicated hosted conversation rules remain;
- no broad universal `Store` trait was introduced;
- shared scenario tests pass against SQLite and isolated PostgreSQL;
- hosted account/revision/idempotency/event behavior remains intact;
- existing local API, CLI, runtime, and Inspector behavior still pass their
  relevant tests.
