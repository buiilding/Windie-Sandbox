# Durable events

## Purpose

Durable events are the saved activity history of a session. They record how
execution progressed: input was queued, model output streamed, a message or
tool result was saved, approval was required, or the session reached a final
outcome.

Windie persists these events so a client does not need to remain connected for
the entire execution. The Inspector, CLI, and notifier can reconnect, replay
events they missed, and then continue following live activity through
Server-Sent Events (SSE).

## Relationship to conversations and sessions

Durable events are related to conversation and session state, but they do not
replace either one:

- The **conversation tree** stores the actual system, user, assistant, and tool
  messages that form model history.
- The **session record** stores the current execution state, such as its
  current message head, status, queue, approval state, and wakeup settings.
- **Durable events** form the chronological activity log explaining how the
  session reached its current state.

An assistant response therefore exists as a message in the conversation tree.
Events can record its streamed text and the ID of the saved message, but the
event log is not another copy of the conversation transcript.

## SQLite storage

Events are stored in their own `session_events` table, separate from the
conversation message tables. Each row contains:

| Column | Meaning |
| --- | --- |
| `id` | An automatically increasing event ID used as a replay cursor. |
| `session_id` | The durable session that produced the event. |
| `event_type` | A stable event name such as `input_queued` or `completed`. |
| `payload` | Event-specific data serialized as JSON. |
| `created_at` | The event time as Unix milliseconds. |

Event IDs increase across the whole database, not separately inside each
session. This gives Windie one ordering for the aggregate event stream while
still allowing events to be filtered and replayed for one session. Consumers
should use the event ID for ordering and resume cursors; timestamps describe
when events were created but are not the ordering authority.

## Event types and payloads

The current durable event types are:

| Event | Saved information |
| --- | --- |
| `input_queued` | Input ID and the resulting queue depth. |
| `input_started` | Input ID and the user-message ID created when execution begins. |
| `wakeup_message_saved` | ID of the durable wakeup message. |
| `assistant_delta` | One streamed assistant-text chunk. |
| `reasoning_delta` | One streamed reasoning-text chunk. |
| `tool_call_delta` | Tool-call position and streamed ID, name, or argument data. |
| `assistant_attempt_reset` | Tells clients to discard streamed output from an abandoned assistant attempt. |
| `assistant_message_saved` | ID of the completed assistant message in the conversation tree. |
| `tool_result_saved` | ID of the saved tool-result message. |
| `waiting_for_approval` | Records that execution paused for a user decision. |
| `completed` | Optional ID of the final assistant message. |
| `failed` | Error text and its recorded causes. |
| `cancelled` | Records explicit cancellation. |

There is no general `session_started` event. `input_started` means a queued
input began execution. Streaming also does not use one “assistant started”
event: each text chunk is persisted as an `assistant_delta`.

Some events need only their type. For example, `waiting_for_approval` and
`cancelled` do not duplicate the full session record in their stored JSON
payload. When the API sends state-changing events over SSE, it can attach a
fresh session or message snapshot for clients to render.

## Main flow

1. A session action or runtime change produces a typed `SessionEvent`.
2. Windie serializes that event and inserts it into `session_events`. Events
   produced by a running executor are accepted only while its execution claim
   is still valid, preventing a cancelled or replaced runner from continuing
   to publish activity.
3. The session manager publishes the saved event to currently connected
   listeners.
4. A reconnecting consumer supplies its last event ID. Windie loads later rows
   from SQLite in ascending ID order before delivering new live events.

The session-specific SSE route replays events for one session. The aggregate
event route uses the same database-wide IDs to merge events from all sessions
and can filter by event type. The notifier uses that aggregate stream to watch
for completed sessions.

For aggregate completion events, the durable event normally stores the final
assistant message ID. The API loads the canonical assistant text from the
conversation tree when delivering the event; it does not copy that full text
into the event row.

## Important invariants

- Durable events describe execution activity, while conversation messages and
  session records remain the authoritative state.
- Every saved event belongs to an existing session.
- Database-wide event IDs provide stable replay order and duplicate
  suppression across reconnects.
- Events written by an active runtime must match its current execution claim.
  A stale runner cannot keep publishing after cancellation or ownership
  transfer.
- Message-save, session-head, lifecycle, and related event updates use
  transactions where they must change together, preventing an event from
  claiming a durable mutation happened when that mutation rolled back.

## Related code

- `src/session/event.rs` — typed event kinds, payloads, and persisted event
  records.
- `src/store/schema.rs` — `session_events` table and indexes.
- `src/store/session.rs` — event insertion, transactional state changes, and
  replay queries.
- `src/session/manager.rs` — live event publication during API-owned session
  execution.
- `src/api/sse.rs` and `src/api/event.rs` — session-specific and aggregate SSE
  serialization.
