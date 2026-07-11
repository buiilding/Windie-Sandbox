# Streaming and Durable Runs

Windie has two related streaming layers:

1. Bifrost model-response streaming into Windie's runtime;
2. Windie run-event streaming from the API to the inspector.

They are not the same stream. Windie parses provider events, persists durable
state, and then exposes a client-oriented event journal.

## Model Stream

Windie's Bifrost client sends an OpenAI-compatible streaming Responses request.
It consumes response bytes incrementally, preserves incomplete UTF-8 sequences
between chunks, splits complete event lines, and decodes response events.

The client maintains one assistant stream state containing:

- visible assistant text;
- assistant metadata;
- partial tool calls keyed by output index;
- finish reason;
- provider usage.

## Runtime Delta Types

The LLM boundary emits three transient delta types to runtime:

- assistant text delta;
- reasoning-summary text delta;
- tool-call delta with index, optional ID/name, and argument fragment.

Tool calls may arrive over several provider events. Windie accumulates ID,
name, and argument text in a stable index map and finalizes complete typed calls
only when the response finishes.

The terminal client writes deltas directly through `RuntimeOutput`. The API run
output converts them into durable run events.

## Assistant Persistence Timing

Streaming deltas are not message nodes. Runtime waits for the Bifrost stream to
finish, then:

1. ends the transient assistant output;
2. finalizes assistant text and metadata;
3. inserts one assistant message below the previous active node;
4. emits `assistant_message_saved` after the store commit;
5. resolves policy-denied or automatic tools as allowed.

If the model stream fails before finalization, Windie does not persist a
partial assistant message.

## Why API Runs Exist

An HTTP response-owned stream would tie model work to one browser connection.
Windie instead creates a backend-owned run before spawning the task.

The run persists:

- run ID and conversation ID;
- lifecycle status;
- optional terminal error;
- ordered event records.

The async task and cancellation handle remain process-only.

## Run Statuses

| Status | Meaning |
| --- | --- |
| `running` | Backend task is expected to be active |
| `completed` | Terminal success event was persisted |
| `failed` | Terminal error event and error text were persisted |
| `cancelled` | User explicitly cancelled active work |
| `interrupted` | A previous process ended before finishing the run |

Only one running action is allowed per conversation.

At API startup, every database run still marked `running` becomes
`interrupted`, because its task cannot survive the process restart.

## Run Events

Every event receives a monotonically increasing sequence number local to the
run.

| Event | Meaning |
| --- | --- |
| `assistant_delta` | Ephemeral visible assistant text |
| `reasoning_delta` | Ephemeral reasoning-summary text |
| `tool_call_delta` | Ephemeral call ID/name/argument progress |
| `assistant_message_saved` | Complete assistant message is durable |
| `tool_result_saved` | One tool output is durable |
| `query_done` | Run completed, with optional final message ID |
| `query_error` | Run failed, with top-level error and cause chain |
| `run_cancelled` | Explicit cancellation completed |

The three delta types are also persisted in the event journal. They can be
replayed after reconnection even though they are not conversation messages.

`assistant_message_saved` and `tool_result_saved` are emitted only after the
corresponding store mutation succeeds.

## Publishing an Event

Publishing performs:

1. serialize the typed event;
2. open the run store;
3. transactionally select the next sequence number;
4. insert the event and update the run timestamp;
5. broadcast the same envelope to live subscribers.

Persistence happens before broadcast, so a subscriber that misses the live
notification can recover it from SQLite.

## Starting a Query Run

The inspector posts model and reasoning overrides to the conversation run
route. The API:

1. rejects a second active run for that conversation;
2. creates the durable `running` record;
3. creates a process broadcast channel;
4. spawns the query task;
5. registers its abort handle;
6. returns the run snapshot immediately.

Approval and denial runs use the same infrastructure with different runtime
actions.

## SSE Replay and Follow

The inspector requests:

```text
GET /api/runs/<run_id>/events?after=<last_sequence>
```

The API subscribes to the live broadcast channel before loading replay history.
That ordering prevents an event from falling between history load and live
subscription.

It then sends:

1. persisted events after the cursor;
2. new broadcast events as they arrive;
3. recovered persisted events if the broadcast receiver reports lag;
4. terminal completion once a terminal event is observed.

When a run is no longer active, the API still replays its persisted history and
then closes the stream.

## Inspector Live State

The inspector tracks:

- active run ID;
- highest received sequence;
- an abort controller for its SSE request;
- one ephemeral pending assistant preview.

Assistant and reasoning deltas append to the preview. Tool-call deltas are
grouped by output index and their argument fragments are concatenated.

On `assistant_message_saved` or `tool_result_saved`, the inspector reloads the
conversation and approvals from the API, then clears the ephemeral preview.
The durable message tree becomes the source of truth.

On `query_done`, it performs one final reload without another token count. On a
stream error, it also reloads without counting so persisted state is still
shown.

## Browser Reload Recovery

When a conversation becomes active, the inspector calls its `active-run` route.
If the API reports a running action, the inspector reconnects from sequence
zero and reconstructs display state from replay plus durable conversation
reload events.

Refreshing the browser therefore does not cancel model or tool work. Restarting
the API process interrupts it.

## Cancellation

The inspector stop action posts to the run cancellation route and aborts its
local SSE request.

Backend cancellation:

1. verifies the run is currently `running`;
2. aborts the registered task when available;
3. persists and broadcasts `run_cancelled`;
4. stores `cancelled` status;
5. removes the process-active entry.

Disconnecting or locally aborting SSE without calling cancellation affects only
the subscriber.

## Terminal Completion and Failure

Successful completion first publishes `query_done`, then marks the run
`completed`, then removes it from the active map.

Failure publishes `query_error` with the complete client-facing cause chain,
marks the record `failed`, and removes the active entry.

The event is persisted before the terminal status change. Subscribers use the
terminal event to stop following.

## Broadcast Capacity and Lag

Each active run uses a broadcast channel with capacity 512. A slow subscriber
may lag beyond that buffer. Lag is recoverable because all events were already
persisted; the SSE layer loads events after the subscriber's last sequence.

## Relevant Code

- `src/llm.rs`
- `src/runtime.rs`
- `src/run.rs`
- `src/api.rs`
- `src/store.rs`
- `dev/windie-inspector/src/lib/windieApi.js`
- `dev/windie-inspector/src/context/WindieContext.jsx`
