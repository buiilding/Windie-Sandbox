# Approvals

## Purpose

<!-- Explain how Windie pauses tool execution for an explicit user decision. -->

## Owns

<!-- Describe approval modes, pending requests, decisions, and resumption. -->

## Does not own

<!-- Distinguish approval policy from provider availability and tool execution. -->

## Main flow

1. <!-- Explain how policy classifies a pending tool call. -->
2. <!-- Explain how a session waits for a matching decision. -->
3. <!-- Explain how approval or denial becomes a durable result. -->

## Important invariants

- <!-- A decision applies only to its intended session and tool call. -->
- <!-- Automatic approval never exposes detached or unavailable tools. -->

## Related code

- <!-- `src/tool/approval.rs` -->
- <!-- `src/tool/policy/` -->
- <!-- `src/operation/session_approval.rs` -->
