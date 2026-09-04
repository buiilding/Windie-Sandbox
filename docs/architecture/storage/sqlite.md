# SQLite storage

Windie's durable local store is SQLite. The default database is
`~/.windie/windie.db`; tests can use an isolated in-memory database. The
`Store` type is the persistence boundary, so the rest of the runtime works
with typed operations instead of knowing about tables and SQL queries.

```text
API / CLI / runtime operations
              │ typed Store methods
              v
       SQLite database
              │
              ├── conversations and message tree
              ├── sessions, queues, claims, and events
              └── tools, components, compactions, and local access
```

## Purpose

SQLite is Windie's local source of truth for state that must survive a client
disconnect, a process restart, or a later API/CLI invocation. It keeps the
conversation tree, session position, queued work, approval state, and replayable
events together so related changes can be committed as one durable operation.

The database is local rather than a remote runtime service. The API and CLI
open the same user database, while the Inspector reads authoritative snapshots
and event streams through the API instead of reading SQLite directly.

## Owns

The SQLite store owns these durable records:

- **Conversations** — conversation identity, title, model, reasoning setting,
  tool approval mode, and the conversation-wide system prompt.
- **Messages** — the parent-linked conversation tree, typed message metadata,
  ordered message parts, and referenced image assets.
- **Sessions** — branch start/current heads, lifecycle status, model settings,
  execution claims, idle-wakeup settings, and the FIFO input queue.
- **Session events** — replayable input, streaming, tool, approval, completion,
  failure, cancellation, and wakeup events with a monotonic database cursor.
- **Compactions** — summaries saved through a specific conversation message.
- **Tool state** — conversation-attached tool schemas, installed component
  lifecycle records, provider tool catalogs, and Chrome DevTools settings.
- **Runtime access** — the singleton hosted-account pairing allowed to use this
  local runtime.

`Store` methods validate typed IDs and ownership before writing. The schema
uses foreign keys and indexes for relationships and common lookups, including
conversation messages, message parents, session events, queued inputs, and
provider state.

Operations that change related rows use SQLite transactions. Examples include
materializing a queued input and advancing the session head, saving a
runtime-produced message with its event, finishing a session at a new head,
and deleting a session's events and exclusive message suffix.

## Does not own

SQLite does not own in-memory execution or presentation:

- The session manager owns live task handles, per-session gates, and broadcast
  channels. SQLite persists the state those tasks must coordinate around, but
  it does not supervise a task or stream an event to a browser by itself.
- The runtime owns model turns, context compilation, approval decisions, and
  tool execution. SQLite stores their durable messages and events; it does not
  call Bifrost or execute an MCP process.
- The API and CLI own transport, authentication, parsing, and presentation.
  They must use the store boundary rather than reconstructing state from local
  caches.
- Package installation and provider process setup belong to the plugin and
  component operations. SQLite stores lifecycle records and discovered
  catalogs; it does not install packages or start provider processes.

## Main flow

1. `Store::open()` opens `~/.windie/windie.db`. `Store::open_at()` creates the
   parent directory when needed, opens the SQLite connection, enables foreign
   keys, configures WAL journaling, uses `synchronous = NORMAL`, sets a
   five-second busy timeout, and runs `PRAGMA optimize` after setup.
2. Startup reads SQLite's `user_version`. A new database receives the current
   schema; the supported additive migration upgrades schema version 25 with
   the session idle-wakeup interval. Databases newer than the supported
   version, unsupported older databases, and existing unversioned databases
   fail closed instead of being guessed or silently rewritten.
3. Conversation, session, tool, and runtime operations call the relevant
   `Store` module. Reads return typed Windie values. Mutations verify that
   referenced conversations, messages, session heads, and tool calls belong to
   the expected owner before changing rows.
4. A multi-row runtime change starts a SQLite transaction with the required
   locking behavior, performs all related writes, and commits once. If a
   validation or write fails, the transaction is not published as a partial
   state. Session execution claims and current-head checks prevent an old
   runner from writing after cancellation or a newer runner's update.
5. Later API or CLI calls reconstruct current state from SQLite: they load the
   conversation tree or selected path, the session and queued inputs, the
   attached tools, and events after a requested cursor. The API may publish
   those events through SSE and the Inspector may cache them, but the database
   remains authoritative.

## Important invariants

- The database schema version is explicit and checked on every open. Windie
  does not run against a newer schema, silently interpret an unsupported old
  schema, or adopt an existing unversioned database.
- `PRAGMA foreign_keys = ON` protects declared relationships. Store-level
  checks add ownership constraints that SQLite cannot express with the schema
  alone, such as requiring a message parent and a selected session head to
  belong to the same conversation.
- The conversation tree is canonical. Windie derives selected root-to-head
  paths from parent links instead of persisting a duplicate linear transcript.
- A runtime message, its session-head movement, and its corresponding saved
  event are committed together when the operation requires all three. Queued
  input materialization and session completion at a new head use the same
  atomic boundary.
- Session execution claims are checked inside the write transaction. A stale,
  cancelled, or transferred runner cannot append a message, append a runtime
  event, or publish a terminal state using an old claim.
- Session event IDs are database-generated and monotonic for the store. The
  `(session_id, id)` index supports replay for one session without making the
  event log the canonical conversation transcript.
- Image assets are referenced by ordered message parts. Deleting messages or
  truncating a branch cleans up assets that no longer have a part reference.
- The runtime-access table is a singleton. A second hosted account cannot
  replace the account that already owns this local database without an explicit
  unlink.
- Provider credentials for LLM inference are managed by Bifrost, not stored as
  Windie conversation or session data. Windie persists only the local runtime
  state it owns.

## Related code

- `src/store/mod.rs` — SQLite connection setup and the persistence boundary.
- `src/store/schema.rs` — schema creation, version checks, migration, tables,
  foreign keys, and indexes.
- `src/store/conversation.rs` — conversation rows and conversation settings.
- `src/store/message.rs` — message tree, ordered parts, image assets, paths,
  mutations, and conversation forks.
- `src/store/session.rs` — sessions, queued inputs, claims, current heads, and
  replayable events.
- `src/store/compaction.rs` — compaction checkpoints.
- `src/store/tool_schema.rs` — conversation-wide attached tool schemas.
- `src/store/component.rs` — installed component lifecycle records.
- `src/store/tool_catalog.rs` — persisted provider-owned MCP tool catalogs.
- `src/store/runtime_access.rs` — singleton hosted-account pairing.
- [Storage overview](README.md) — how conversations, sessions, and events fit
  together.
- [Conversation tree](conversation-tree.md) — durable message structure.
- [Session lifecycle](session-lifecycle.md) — durable execution states and
  transitions.
