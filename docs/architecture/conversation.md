# Conversation Architecture

Windie stores each conversation as a tree of message nodes plus settings that
apply to the conversation as a whole. Each root-to-node path is one possible
conversation context. The tree keeps all of those contexts, including inactive
branches, without duplicating their shared ancestors.

## Conversation-Level State

The conversation row stores:

- a stable conversation ID;
- the default model;
- optional reasoning effort;
- the active message ID;
- an optional system prompt;
- tool approval mode;
- creation and update timestamps.

Attached tool schemas and compaction checkpoints are persisted in related
tables. The system prompt, attached schemas, model, and reasoning settings are
not message nodes and are not duplicated on every branch.

## Tree Shape

Each message has an optional `parent_message_id`. A message without a parent is
a root. Multiple messages may share a parent, which creates branches:

```text
user A
  |
  +-- assistant B
  |      |
  |      +-- user C
  |
  +-- assistant D
         |
         +-- user E  <-- active
```

There may be more than one root because deleting a root splices its children to
`NULL`. The tree is therefore technically a forest, although normal insertion
continues from one active chain.

Messages carry one of four typed roles: `system`, `user`, `assistant`, or
`tool`. Normal persisted history uses user, assistant, and tool nodes. The
conversation system prompt is normally stored separately and synthesized only
when context is built.

## Selected Path

The conversation stores one `active_message_id`. Windie finds that message and
walks through parent links to produce the user-selected path.

For the tree above, the active path is:

```text
user A -> assistant D -> user E
```

That path is the default context for the next runtime operation.
`assistant B -> user C` remains durable and can be selected later.

### Setting the Path

Clients set a path by activating its final message. They do not submit or
persist the whole path array. The backend verifies that the message belongs to
the conversation, stores it as `active_message_id`, and derives all ancestors
when the path is loaded.

Activating a node:

- does not copy messages;
- does not remove other branches;
- does not query the model;
- updates the conversation timestamp.

An empty conversation has no active message and therefore an empty selected
path.

## Runtime Path

When an operation starts, runtime captures the selected head in an
`ExecutionCursor`. The run then owns that cursor and advances it after each
persisted assistant or tool message. Model context is always rebuilt from the
run cursor, not from the conversation's current `active_message_id`.

The selected head and run head normally advance together. Each runtime insert
updates the selected head only when it still equals that insert's captured
parent. If the user selects an older node or creates another branch while the
run is waiting for a model or tool, the selected head moves but the run keeps
using its original branch:

```text
        +-- C -> assistant -> tool -> assistant   run path
A -> B
        +-- D                                     selected path
```

Setting a path, inserting a branch, or forking a conversation therefore does
not redirect or stop an in-flight agent loop. No path array is persisted for
the run; its head plus the tree's parent links are sufficient.

## Inserting Messages

Normal insertion always uses the current active message as the new node's
parent. The inserted node then becomes active.

```text
active B + insert C -> B is parent of C -> C becomes active
```

This rule is what creates a branch after path selection:

1. activate an older node;
2. insert a new message;
3. the new message becomes another child of that older node.

Direct insertion supports user and assistant roles. The CLI parser accepts the
word `tool`, but the shared operation rejects it. Tool messages must be created
through tool execution or denial so the store can validate their call ID.

### Text and Images

A simple message stores visible `content`. Multipart user messages also store
ordered text/image parts. Image paths or API-supplied bytes are loaded and
copied into durable SQLite image assets; the original path is not retained.

Multipart direct insertion is restricted to user messages. Tool execution can
persist multipart tool results through its dedicated validated path.

## Modifying Messages

Updating a message replaces its visible text in place. It does not:

- create a sibling or branch;
- change the role or parent;
- delete descendants;
- replace metadata;
- move the active pointer.

For multipart messages, Windie removes old text parts, preserves image parts,
normalizes their positions, and optionally inserts the replacement text first.

Updating clears all conversation compactions because an existing summary may
describe text that no longer exists.

While a run owns the conversation, editing, removal, truncation, and other
destructive mutations are rejected. Activating an existing path, inserting a
branch, and forking remain available because they do not invalidate the path
the in-flight operation captured.

## Removing One Message

Normal removal is a splice delete. Direct children are reparented to the
removed node's parent, while deeper descendants keep their links:

```text
before: A -> B -> C
remove B
after:  A -> C
```

If `B` had several children, every direct child is promoted. Removing a root
makes its direct children roots.

The active pointer behaves as follows:

- if the active message survives, it remains active;
- if the active message is deleted, Windie prefers the deleted node's parent;
- if there is no parent, it may select the first promoted child;
- if neither exists, the conversation becomes empty.

Removal also clears compactions and deletes image assets no remaining message
part references.

Assistant tool-call nodes and their outputs have an atomic group-deletion rule
described in [execution.md](execution.md).

## Removing a Conversation

Conversation removal transactionally clears the active pointer, deletes
compactions, attached tool schemas, messages, and orphaned images, then deletes
the conversation row.

Runtime runs, events, and execution claims cascade with their conversation.

## Truncating

Truncation keeps the selected checkpoint message and recursively deletes every
descendant below it.

```text
A -> B -> C -> D
truncate after B
A -> B
```

Every branch below `B` is removed, not only the selected active branch. Branches
elsewhere in the conversation remain.

If the active message was among the removed descendants, `B` becomes active.
If the active message was elsewhere, it remains active. Truncation clears
compactions and orphaned images.

## Forking

Forking creates a new conversation from the root-to-selected-message path. It
does not create another branch in the same conversation.

Copied messages receive new IDs and rebuilt parent links. Copied image content
receives independent asset IDs. The selected copied node becomes active.

The fork copies:

- path messages, parts, and metadata;
- default model;
- reasoning effort;
- tool approval mode.

It currently does not copy:

- system prompt;
- attached tool schemas;
- compaction checkpoints.

## How the Model Sees a Conversation

Windie, not Bifrost, builds model context. `ContextBuilder` is the single
projection boundary for both stored messages and inspection views. Inspection
uses the selected path. Inference uses the explicit runtime path. The builder
optionally replaces history through an applicable compaction with a synthetic
summary and prepends the conversation system prompt. Attached tool schemas are
loaded separately and sent with the request.

The complete tree remains in storage. Bifrost receives only the already
flattened active context.

Inspection and tree commands load image metadata only: asset ID, MIME type, and
byte count. Inference still loads full bytes for images on the runtime path, and
the image API remains the binary display boundary.

At operation start, Windie transactionally snapshots model, reasoning, system
prompt, compaction, approval mode, and attached tools. Those settings remain
stable through automatic tools and continuation turns. The run cursor evolves
as runtime persists messages, while user path selection remains independent.
Settings changed during a run apply to the next run.

## Core Invariants

- A message parent must belong to the same conversation.
- The active message must belong to its conversation.
- New normal messages append below the active message.
- Tool outputs cannot be inserted through the generic message path.
- Mutations that invalidate summaries clear compactions.
- Tree mutation must not leave dangling tool calls or tool outputs.

## Relevant Code

- `src/conversation.rs`
- `src/context.rs`
- `src/store.rs`
- `src/store/messages/`
- `src/operation.rs`
- `src/store/tests.rs`
