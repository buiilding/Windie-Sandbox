# Execution claims

An execution claim is the durable fencing token for one attempt to run a
session. It is stored on the session row as an execution owner (`api` or
`cli`) and a fresh claim ID. The owner identifies the client surface; the ID
is the value that protects every run-owned write.

```text
session is runnable
       │
       v
conditional SQLite claim
       │ fresh owner + claim ID
       v
     Running
       │
  runtime writes and events carry the same claim
       │
       v
finish, wait for approval, fail, or cancel
       │
       └── claim is cleared only after the owner stops
```

## Purpose

Sessions can be executed repeatedly, and API and CLI processes can try to
work with the same durable session. A status such as `Running` is not enough
to identify which runner may write: an older runner could continue after a
client cancelled it or after another execution attempt was started.

Every execution attempt therefore receives a new claim ID. SQLite accepts
run-owned writes only when the session is still `Running` with the same owner
and claim ID. This fences stale runners so they cannot append messages or
events, move the session head, or overwrite a newer terminal state.

## Owns

Execution claims own the durable hand-off between a session and one active
execution attempt:

- atomically acquiring a fresh claim when the requested start condition still
  matches the session;
- recording whether the claim belongs to the API or CLI execution surface;
- validating the exact owner and claim ID before runtime messages, tool
  results, streamed events, and terminal transitions are persisted; and
- clearing the claim when the run finishes, waits for approval, fails, or the
  cancelled runner has stopped.

The claim start conditions are typed. A caller can start a generally runnable
session, require a specific current head, resume an approval-waiting session,
or start an eligible idle or manual wakeup. The SQLite update checks the
corresponding status, head, activity, and wakeup conditions instead of leaving
those checks to each client.

## Does not own

An execution claim is not the session identity. The `SessionId` identifies the
durable branch over the conversation tree; a claim identifies only one attempt
to advance that branch.

It is also not a process ID, OS lock, or proof that a process is still alive.
The owner value only records whether the API or CLI surface acquired the
claim. The durable owner coordinates separate processes; the session manager's
in-process task map and per-session mutex handle local scheduling separately.

The claim does not choose model context, execute tools, own conversation
messages, or define the session lifecycle. Those responsibilities remain with
the runtime, conversation tree, and session components.

## Main flow

1. An API, CLI, or wakeup workflow asks the store to start a session with a
   typed `SessionExecutionStart` condition. The store creates a fresh UUID
   claim and performs one conditional SQLite update. It succeeds only when no
   other claim is present and the session still satisfies that condition; the
   session moves to `Running`.
2. The runner carries the returned `SessionExecutionClaim`. Runtime messages
   and tool results are committed only when the session still has the same
   owner and claim ID. Those message commits also require the expected current
   head, so an old runner cannot append to a branch that another runner has
   already advanced.
3. Streamed runtime events use the same claim check. A cancelled or transferred
   runner therefore receives a conflict instead of publishing more events.
4. On completion or an approval pause, Windie updates the session status and,
   when needed, its current head and terminal event, then clears the claim. A
   failed run follows the same claim-checked status transition. Cancellation
   deliberately leaves the claim in place while the runner unwinds; the owner
   releases it only after the task has stopped.
5. If a duplicate API or CLI runner tries to start while a claim is present,
   the conditional update affects no rows and the start is rejected. If an old
   runner later attempts a write with its previous claim ID, the claim check
   rejects it even if the owner kind is the same.
6. When the API process starts, it marks interrupted `Running` sessions with
   API-owned claims as failed and clears those claims. CLI-owned claims are not
   treated as API interruptions because the CLI process may still be running.

## Important invariants

- Every execution attempt receives a fresh claim ID, even when the same API or
  CLI surface runs the session again.
- A session has at most one persisted owner and claim ID. The owner and ID are
  stored together; an incomplete pair is treated as invalid state.
- Claim acquisition is conditional on the requested start state. It cannot
  claim a session that is already running or waiting for approval, has changed
  its selected head, or is no longer eligible for the requested wakeup.
- Every run-owned message, tool result, streamed event, and terminal transition
  (completion, approval pause, or failure) validates the exact owner and claim
  ID. External cancellation is the deliberate exception: it records the
  cancellation first, then the cancelled runner releases its claim after it
  stops. A session status alone never grants run-owned write permission.
- Cancellation is durable and cannot be resurrected by a runner that finishes
  later. The cancelled claim remains until that runner acknowledges the stop.
- Successful completion or an approval pause releases the claim as part of the
  state transition. Completion at a new head and its completion event are
  committed together so another runner cannot start from an unpublished head.
- API restart recovery does not replay an interrupted model or tool request.
  It records the API-owned run as failed because replay could duplicate
  provider work or an external tool action.

## Related code

- `src/session/model.rs` — typed session statuses, owner kinds, start
  conditions, claims, and claimed-session results.
- `src/session/id.rs` — `SessionExecutionClaimId`, including fresh UUID
  creation and its persisted representation.
- `src/store/schema.rs` — `execution_owner` and `execution_claim_id` columns
  on the durable `sessions` table.
- `src/store/session.rs` — conditional acquisition, claim validation for
  messages and events, terminal transitions, cancellation release, and claim
  lookup.
- `src/session/manager.rs` — API execution ownership, in-process scheduling,
  cancellation, and API-restart recovery.
- `src/operation/session.rs` — shared API/CLI execution and claim-aware
  message, event, success, and failure workflows.
