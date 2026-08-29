# Session lifecycle

## Purpose

A session is Windie's durable execution record for one branch of a conversation.
It does not copy the transcript. Instead, it points to a start head and a
current head in the shared conversation tree, while recording whether Windie
is ready to work, running, waiting for approval, finished, failed, or
cancelled.

The session lifecycle makes background work explicit. The conversation tree
answers what was said; the session answers whether Windie is currently working
on that branch and what must happen before work can continue.

## Owns

Sessions own the durable execution state for one branch: start and current
heads, status, model and reasoning settings, queued input, approval state,
wakeup settings, execution claims, and replayable execution events.

A session can run more than one runtime turn. New user input, a continuation,
or an eligible wakeup can start another execution attempt on the same session.

## Does not own

The conversation tree owns messages, tool calls, tool results, and branching.
The session points to those messages but does not store another copy of them.

The runtime turn owns the active model-and-tool loop. The session manager owns
starting that loop in the API process, coordinating concurrent requests,
queuing inputs, recovery after restart, and background wakeups.

## Main flow

1. A client selects a conversation and message head. SQLite resolves the
   unique session already at that branch head, creates one when none exists, or
   rejects an ambiguous result rather than guessing.
2. A new session starts in `Ready`. A user message may be accepted immediately
   or queued when that session is already active. The session manager claims
   the session before running it and moves it to `Running`.
3. During a runtime turn, assistant messages and tool results are saved into
   the conversation tree. Each saved result advances the session's current
   head so the next turn begins from the latest durable branch position.
4. If a tool needs user permission, the session becomes `WaitingForApproval`.
   Approving or denying that specific tool call resumes the session and starts
   a fresh execution claim.
5. When the runtime turn has no more tool calls, the session becomes
   `Completed`. A provider or runtime error makes it `Failed`; explicit stop
   makes it `Cancelled`.
6. A later continuation, queued input, or eligible wakeup can claim the same
   session for another runtime turn. A new session is needed only when the
   selected conversation head has no matching session branch.

```text
selected conversation head
           │
           v
resolve existing session or create one
           │
         Ready
           │
  input, continuation, or wakeup
           │
           v
claim execution → Running → runtime turn advances the current head
                    │
          ┌─────────┼──────────────────┐
          v         v                  v
       Completed  Waiting approval   Failed / Cancelled
                      │
             approve or deny tool
                      │
                      └──────────────> Running
```

## Important invariants

- A session is a durable branch record, not a duplicate linear transcript.
  Its start and current heads refer to messages in the conversation tree.
- SQLite, not the browser, resolves a conversation head to a session. This
  prevents clients from guessing ownership from stale cached session lists.
- Each active execution receives a fresh execution claim. Every assistant
  message, tool result, event, and terminal transition must present that
  claim, so an old or duplicate runner cannot write after cancellation or a
  newer execution has taken ownership.
- `Running` and `WaitingForApproval` sessions protect their active message path
  from destructive tree mutations.
- An API restart does not automatically replay a lost model or tool request.
  Interrupted sessions are recorded as failed because replay could duplicate
  provider work or an external tool action. Their already-saved conversation
  state remains available for an explicit continuation.

## Execution events

Execution events are a separate durable timeline for the activity of a
session. They record events such as input queued or started, streamed assistant
or reasoning deltas, tool-call deltas, messages and tool results saved,
approval waits, completion, failure, and cancellation.

The conversation tree remains the canonical transcript. Events describe how
the transcript was produced and how the session changed over time. Windie uses
them for live streaming, reconnecting clients, notifications, recovery, and
inspection.

## Related code

- `src/session/model.rs`: session records, statuses, claims, and durable event
  types.
- `src/session/manager.rs`: API-owned session execution, queues, recovery,
  and wakeup scheduling.
- `src/store/session.rs`: SQLite session resolution, input queues, head
  updates, claims, and event persistence.
- `src/operation/session.rs`: shared API and CLI lifecycle operations.
- `src/session/event.rs`: replayable execution-event definitions.
- `src/runtime/turn.rs`: runtime turn executed for a claimed session.
