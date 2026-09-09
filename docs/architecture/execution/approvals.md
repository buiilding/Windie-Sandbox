# Approvals

## Purpose

Windie detects tool calls in an assistant message from the LLM. Before giving
one to a tool provider for execution, Windie checks that the tool is attached,
that a provider can execute it, and what approval mode is selected for the
conversation.

With `manual` approval, the session pauses and the user must confirm or deny
that specific tool call in the Inspector or through the CLI. With
`auto_approve_attached`, the user does not have to confirm an attached,
available tool call; Windie executes it automatically.

## Owns

- The conversation-level approval modes: `manual` and
  `auto_approve_attached`. New conversations default to `manual`.
- Pending approval requests, including the session, assistant message, tool
  call, and reason that require a decision.
- Pausing a session at `WaitingForApproval` until its pending tool call is
  approved or denied.
- Inspector/API approval routes:
  `GET /api/sessions/{session_id}/approvals`,
  `POST /api/sessions/{session_id}/approvals/{tool_call_id}/approve`, and
  `POST /api/sessions/{session_id}/approvals/{tool_call_id}/deny`.
- CLI approval commands:
  `windie run approvals <session_id>`,
  `windie run approve <session_id> <tool_call_id>`, and
  `windie run deny <session_id> <tool_call_id>`.
- Turning either decision into a durable tool result and continuing the session
  when the next step is allowed.

## Does not own

- Which tool call the model requests.
- Tool attachment, provider installation, provider health, or provider
  credentials.
- Tool execution itself. The registered provider or built-in implementation
  performs the call after policy allows it.
- Durable conversation truth outside the approval and tool-result records.

## Main flow

1. The model returns an assistant message containing one or more tool calls.
   Windie saves that assistant message and selects the next unresolved call in
   the order requested by the model.
2. Windie finds the attached tool and checks whether its provider can execute
   it. An unattached tool, unavailable provider, or missing executor is denied
   without waiting for user approval.
3. Windie reads the conversation's approval mode:
   - `manual` returns a pending approval request and moves the session to
     `WaitingForApproval`.
   - `auto_approve_attached` allows an attached, executable tool call to run
     immediately.
4. In manual mode, the Inspector or CLI approves or denies the matching session
   and tool-call IDs. An approval executes the call through the registered
   provider; a denial creates a failed tool result instead.
5. Windie saves the successful or failed tool result, then continues with the
   next unresolved call or the next model turn.

## Important invariants

- Approval mode is stored on the conversation and defaults to `manual`.
- A decision applies only to its intended session and tool-call ID. A decision
  for a later call cannot skip an earlier unresolved call from the same
  assistant message.
- `auto_approve_attached` is not unrestricted execution. It does not allow
  detached tools, unavailable providers, or missing executors.
- In `manual` mode, every executable model-requested tool call requires an
  explicit decision, including Windie-owned built-in tools.
- Every approved, denied, provider-failed, or policy-denied call becomes a
  durable tool result linked to the original tool-call ID.
- Approval delivery never changes which tools are attached to a conversation;
  it only decides whether an already-resolved executable call may run.

## Related code

- [`src/tool/approval.rs`](../../../src/tool/approval.rs) — approval modes and
  pending approval request types.
- [`src/tool/policy/mod.rs`](../../../src/tool/policy/mod.rs) — `Allow`, `Ask`,
  and `Deny` decisions.
- [`src/runtime/tool_execution.rs`](../../../src/runtime/tool_execution.rs) —
  pending-call order, policy checks, execution, and result persistence.
- [`src/operation/session_approval.rs`](../../../src/operation/session_approval.rs)
  — approval listing, approval/denial workflows, and session continuation.
- [`src/api/session_approval.rs`](../../../src/api/session_approval.rs) — API
  handlers used by the Inspector.
- [`src/cli/adapter/session.rs`](../../../src/cli/adapter/session.rs) — CLI
  approval commands.
