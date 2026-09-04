# Session lifecycle

A session is Windie's durable record for advancing one branch of a
conversation. It points to a start head and a current head in the shared
conversation tree, while its status records whether that branch can run, is
running, is waiting for permission, or has reached a terminal result.

## Purpose

The lifecycle makes background work explicit and durable. A client can
disconnect while a session is running, reconnect and replay its events, or
continue a completed or failed session from its latest saved head.

The conversation tree answers what was said. The session answers where
execution is operating, which inputs are waiting, and whether another runtime
attempt may claim the branch.

## Owns

The session owns its durable execution state: conversation and start/current
message heads, model and reasoning settings, lifecycle status, queued inputs,
approval position, idle-wakeup settings, execution claims, and replayable
events.

The lifecycle states are:

| State | Meaning |
| --- | --- |
| `Ready` | The session has been created and has not started an active execution attempt. |
| `Running` | One API or CLI runner holds an execution claim and may advance the session. |
| `WaitingForApproval` | A tool call at the current head needs a user decision before execution can continue. |
| `Completed` | The runtime reached a response with no further automatic tool call. |
| `Failed` | The runtime or its provider failed, or an API restart interrupted the run. The saved branch remains available for an explicit continuation. |
| `Cancelled` | A client explicitly stopped the session. The cancelled runner must stop before another attempt can claim it. |

`Running` is not permanent. A completed, failed, or cancelled session can be
claimed again by a later continue or query operation, and an approval-waiting
session can resume after the relevant approval decision or policy change.

## Does not own

The conversation tree owns message nodes, parent links, tool calls, tool
results, and branching. A session points to the selected branch heads; it does
not copy the conversation transcript.

The runtime owns the active model-and-tool loop. It produces assistant and
tool-result messages through the session persistence boundary, but it does not
define the durable session state by itself.

The session manager owns API-process task supervision, in-process scheduling,
live event delivery, cancellation, approval resumption, queued-input handoff,
and idle-wakeup scheduling. The durable session record remains the authority
when more than one process can access the same SQLite store.

## Main flow

1. A client selects a conversation and message head. SQLite resolves the
   unique session currently ending at that head, creates a new `Ready` session
   when none exists, or rejects the request when more than one session matches.
   Creating a branch does not start model execution.
2. A query, continuation, or eligible wakeup asks the store to claim the
   session. The claim checks that the requested status, selected head, and
   wakeup conditions still match, then moves the session to `Running`. Each
   attempt receives a fresh execution claim; see
   [Execution claims](execution-claims.md).
3. A new user query is saved under the current head before the runtime starts.
   If the session already has a task running in the same API process, the
   input is stored in the durable FIFO queue and an `InputQueued` event is
   emitted. A query that finds a running session owned by another process is
   rejected instead of starting a second runner.
4. While `Running`, the runtime loads the selected path, queries the model, and
   saves assistant messages and tool results. Each saved message advances the
   session's current head and records a replayable event. If a tool call needs
   permission, the runtime returns `WaitingForApproval` at that head. The
   claim is released, but the pending tool call and its branch remain durable.
5. Approving or denying the pending tool call starts a new claimed execution,
   saves the corresponding tool result, and continues the same branch. If no
   more automatic tool calls remain, the session becomes `Completed`.
6. A provider or runtime error changes the session to `Failed` and records a
   failure event. An explicit stop changes it to `Cancelled`; the cancelled
   runner is fenced until it stops and releases its claim. A runner that tries
   to finish after cancellation cannot change the session back to `Completed`.
7. When a run completes, the API manager checks for queued inputs. It claims
   the session again, materializes the oldest queued input under the latest
   head, and starts the next run. Queued inputs are durable, so they survive a
   client disconnect or API task handoff.
8. An enabled idle wakeup or an explicit **Wake now** request follows the same
   claim-and-run path. An idle wakeup first appends a durable wakeup message;
   explicit wakeup records the request as user activity and postpones the next
   idle deadline.
9. On API startup, `Running` sessions with API-owned claims are marked
   `Failed` with an interruption error. Windie does not replay the lost model
   or tool request automatically because repeating provider work or an
   external action could have side effects. Queued inputs and saved messages
   remain available for an explicit continuation.

```text
conversation head
       │
       └── resolve existing session or create Ready
                         │
        query / continue / wakeup claims session
                         │
                         v
                      Running
                    /    |     \
                   v     v      v
          Completed  Waiting  Failed
                         │
               approve or deny tool
                         │
                         └──────> Running

               explicit stop from an active run
                              │
                              v
                         Cancelled
```

## Important invariants

- A session has one durable current head. Runtime-produced messages and tool
  results advance that head only through a claim-checked transaction.
- Only one execution attempt may own a session at a time. A fresh claim is
  required for every run, including approval resumes and queued-input
  handoffs.
- `Running` and `WaitingForApproval` sessions protect the messages on their
  current path from destructive conversation-tree mutations. A session cannot
  be deleted while it is in either state.
- A session waiting for approval cannot accept another user query or start a
  normal continuation. The approval decision must target the pending tool call
  at the session's current head.
- Inputs queued during an active API task are ordered by creation time and
  materialized one at a time. Materializing the input, advancing the head, and
  removing the queue row share one SQLite transaction.
- Completion at a new head and its completion event are committed together.
  Stale or cancelled runners cannot publish a later head or terminal status.
- Cancellation is durable. A runner that finishes after the stop request cannot
  resurrect the session, and the claim is released only after that runner has
  stopped.
- API restart recovery marks interrupted API-owned `Running` sessions as
  `Failed` and does not automatically replay their model or tool work. Already
  persisted messages, events, and queued inputs remain durable.
- The session is the durable lifecycle authority; browser state, live
  broadcast channels, and in-process task maps are views or coordination aids,
  not a replacement for the SQLite record.

## Related code

- `src/session/model.rs` — session fields, lifecycle statuses, wakeup cadence,
  execution-start conditions, and next-wakeup calculation.
- `src/store/session.rs` — session creation and head resolution, queued-input
  persistence, claim-checked state transitions, and durable event writes.
- `src/session/manager.rs` — API task supervision, cancellation, approval
  resumption, queued-input handoff, idle wakeups, and restart recovery.
- `src/operation/session.rs` — shared API/CLI execution, approval, completion,
  and failure workflows.
- `src/operation/session_cli.rs` — CLI-owned session execution through the same
  lifecycle and claim boundary.
- `src/runtime/turn.rs` — model-turn advancement and the completed versus
  approval-waiting runtime outcomes.
- `src/session/event.rs` — replayable session-event definitions used by live
  streams and durable replay.
- [Sessions](sessions.md) — session data model overview.
- [Execution claims](execution-claims.md) — fencing and cross-process
  execution ownership.
