# Conversation tree

## Purpose

A conversation is Windie's durable history of messages. It is stored as a
parent-linked tree rather than as one mutable linear transcript.

This allows a user or runtime to continue from an earlier message without
overwriting what was already explored. Branches inside one conversation share
their ancestor messages, so the shared history is stored once. Selecting a
different branch head gives the model a different history and can therefore
produce a different continuation.

For example, the two paths below share `A` and `B`, but have different model
histories after `B`:

```text
              A
              |
              B
             / \
            C   D

path to C: A, B, C
path to D: A, B, D
```

One correction to an easy misunderstanding: sharing happens *within one
conversation*. Windie's explicit **fork conversation** operation creates a
new conversation, copies the selected root-to-head path into it, and assigns
new message IDs. It does not make two conversations share the same stored
message nodes.

## Owns

The conversation tree owns the durable message structure for one conversation:

- each message's identity, role, text or ordered parts, metadata, and optional
  parent message ID;
- the parent-child relationships that define branches;
- the root-to-selected-head path used as the persisted history for one model
  request; and
- structural edits to that tree: inserting, updating, removing, truncating,
  and copying a selected path into a forked conversation.

Messages may have the roles `system`, `user`, `assistant`, or `tool`. A
`role: tool` message is special: the store only permits it through the
tool-result insertion path, where it is checked against the assistant tool
call it answers.

## Does not own

The tree is durable history, not the whole runtime. Nearby components own
different responsibilities:

- A **session** owns serialized execution at a selected head, its current
  position, status, queued inputs, approvals, and events. It points into the
  tree; it does not copy the messages.
- The conversation record owns tree-wide settings such as the user-owned
  system prompt, model settings, and attached tool schemas. These are not
  branch-local message nodes.
- `runtime/context.rs` compiles the exact model request. It combines the
  selected persisted path with the system prompt, any applicable compaction,
  attached tools, and ephemeral runtime capabilities.
- The Inspector turns the stored tree into a visible graph and transcript. Its
  layout and selected UI node do not change the stored tree.

## Main flow

1. A user, runtime, or approved tool result inserts a message under an
   explicit parent message. A message with no parent is a root. The store
   verifies that a specified parent belongs to the same conversation.
2. To run or inspect one branch, a caller supplies a selected head message ID.
   The store follows that message's parent links back to a root and returns
   the messages in root-to-head order. This is the selected path.
3. `runtime/context.rs` uses that path as the persisted conversation history
   for the request. It does not send unrelated sibling branches to the model.
4. When the model responds, its assistant message is saved as a child of the
   selected head. The session advances its current head in the same SQLite
   transaction, so a stale runner cannot append to the wrong branch.
5. To explore an alternative, insert a new child under an earlier message and
   select that new branch's head. The earlier ancestor nodes remain shared.

The database schema permits more than one root message in a conversation. In
that unusual case, the stored structure is technically a small forest, but any
selected head still has one unambiguous chain of ancestors.

## Important invariants

- The tree is canonical storage. A selected path is derived from parent links;
  Windie does not persist a duplicate linear transcript for every branch.
- A message may only name a parent in the same conversation. A selected path
  cannot cross into another conversation or include a sibling branch.
- The exact model-visible context is compiled by `runtime/context.rs`. The
  selected path is its persisted-history input, but the conversation-wide
  system prompt and tool schemas, compaction, and ephemeral runtime metadata
  are handled separately.
- A running or approval-waiting session protects every message on its current
  path from replacement, removal, or truncation. Adding a new child branch is
  allowed because it does not modify that protected path.
- Removing an ordinary message is a splice: its direct children are reparented
  to its parent so later descendants survive. Removing a tool-call assistant
  message or one of its tool-result messages removes that whole tool-call
  group, preventing dangling calls or results in model context.
- Truncating after a message deletes all of its descendants. Updating,
  removing, or truncating messages clears saved compactions because their
  summaries may no longer describe the visible history.
- If a non-active session points at deleted messages, Windie repairs its heads
  to the nearest surviving ancestor when possible. The session is not allowed
  to silently retain a deleted message ID.

## Related code

- `src/conversation/message.rs` — message roles and the in-memory message
  shape.
- `src/store/schema.rs` — SQLite message table and parent-link indexes.
- `src/store/message.rs` — path loading, tree mutation, tool-result checks,
  truncation, and conversation forks.
- `src/store/session.rs` — active-session path protection and atomic runtime
  message/head persistence.
- `src/runtime/context.rs` — selected-path loading and model-context
  compilation.
- `src/operation/inspection.rs` — read-only tree, selected-path, and
  root-to-leaf inspection views.
- [`docs/conversation-tree-and-paths.md`](../conversation-tree-and-paths.md)
  — deeper explanation of path loading and its performance trade-offs.
