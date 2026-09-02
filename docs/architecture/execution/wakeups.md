# Wakeups

## Purpose

To make the AI proactive, Windie cannot wait only for the user to type a
message and start a query. A wakeup is an event that causes Windie to resume
work on a durable session.

The primary wakeup is still user-authored input, so the AI reacts to what the
user asks. Other wakeups let Windie do proactive work, such as reviewing the
current context after a period of inactivity. In the future, the same
primitive can support tasks such as auditing, preparation, or other supporting
and clerical work.

## Owns

- Treating ordinary user input, automatic work, and other activation sources as
  requests that enter the same session runtime path.
- The currently supported durable task wakeup kinds:
  - `manual`: the user explicitly asks an existing session to wake now;
  - `idle`: an enabled session reaches its configured idle interval after user
    activity.
- The fixed prompts used by the current manual and idle wakeups. These prompts
  tell the model why it was woken and let it review the current context before
  deciding what useful work to do.
- Persisting an idle or manual task wakeup as a provider-facing `user` message
  with `WakeupMetadata`. The message content is different from a human-authored
  message, and its metadata records that Windie created it and which wakeup
  kind caused it.
- Scheduling idle wakeups for sessions with `keep_awake` enabled. The API
  scheduler checks the stored user-activity time, previous idle completion, and
  the session's configured interval before claiming a wakeup.

## Does not own

- The session's durable branch, execution claim, or lifecycle state. The session
  manager owns those boundaries and starts the runtime after a wakeup is
  accepted.
- Context compilation, model queries, tool execution, or approval policy.
- The definition of future external triggers. Schedules, file events, browser
  events, and system events are planned wakeup sources, not current
  implementations in this module.
- Approval and denial decisions as transcript messages. They are also called
  wakeups internally because they resume a waiting session, but they are
  session-targeted control inputs and do not append a user-role wakeup message.

## Main flow

1. A wakeup originates from ordinary user input, an explicit manual wakeup, an
   eligible idle schedule, or a future event source.
2. Windie targets an existing durable session and claims it using the session
   manager's execution rules. Idle wakeups also re-check the persisted
   eligibility conditions in SQLite so a concurrent user action can win the
   race.
3. For a manual or idle task wakeup, Windie appends a durable message with the
   model-facing `user` role and `WakeupMetadata` (`manual` or `idle`). This lets
   the model receive the request through the normal conversation path while
   storage and the Inspector can distinguish it from human input.
4. The session manager runs the normal runtime loop: compile context, query the
   model, resolve tools and approvals, save results, and finish or pause the
   session.
5. The resulting wakeup message and execution events remain available through
   the durable conversation and session-event records.

## Important invariants

- A wakeup uses the same session execution path as normal user input after it
  is accepted.
- Runtime-generated task wakeups use the provider-facing `user` role so the
  model can act on them, but `WakeupMetadata` preserves their Windie-created
  provenance for storage and presentation.
- The wakeup kind changes why the session was activated; it does not bypass
  context compilation, tool attachment, approval policy, or other permission
  boundaries.
- Idle wakeups require an enabled session and a completed cooldown interval.
  Explicit user activity postpones the next idle wakeup.
- Approval and denial controls resume a waiting session without pretending that
  a human-authored message was added to its transcript.
- The current implementation supports `manual` and `idle` task wakeups. New
  proactive sources should enter through the same typed boundary rather than
  inventing a separate runtime path.

## Related code

- [`src/runtime/wakeup.rs`](../../../src/runtime/wakeup.rs) — wakeup control
  types and current manual/idle prompts.
- [`src/conversation/assistant_metadata.rs`](../../../src/conversation/assistant_metadata.rs)
  — `WakeupKind` and `WakeupMetadata` attached to runtime-created messages.
- [`src/session/manager.rs`](../../../src/session/manager.rs) — explicit and
  idle wakeup scheduling, durable message creation, and runtime activation.
- [`src/store/session.rs`](../../../src/store/session.rs) — durable wakeup
  message persistence, eligibility checks, and session claims.
- [`src/operation/session.rs`](../../../src/operation/session.rs) — shared
  execution commands for idle, manual, approval, and denial wakeups.
- [`src/api/session.rs`](../../../src/api/session.rs) — API routes for explicit
  wakeups and idle-wakeup settings.
