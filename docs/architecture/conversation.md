# Conversation Architecture

Windie stores each conversation as a tree of message nodes plus settings that
apply to the conversation as a whole. Runtime uses one selected path through
the tree as model context.

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

## Active Path

The conversation stores one `active_message_id`. Windie finds that message and
walks through parent links to produce the root-to-active path.

For the tree above, the active path is:

```text
user A -> assistant D -> user E
```

Only that path is sent to the model. `assistant B -> user C` remains durable
and can become active later.

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

An empty conversation has no active message and therefore an empty active path.

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

Current behavior: the backend and inspector allow visible text on assistant
tool-call nodes and tool-output nodes to be edited. Their linking metadata is
preserved. This is different from deletion, which treats a tool-call group as
one unit.

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

Runtime run rows are not currently included in that deletion, and their
conversation foreign key has no delete cascade. A conversation with durable run
records therefore cannot currently be removed; the transaction fails and rolls
back. This is current behavior, not a conceptual ownership rule.

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

Windie, not Bifrost, builds model context. It loads the active path, optionally
replaces history through an applicable compaction with a synthetic summary,
and prepends the conversation system prompt. Attached tool schemas are loaded
separately and sent with the request.

The complete tree remains in storage. Bifrost receives only the already
flattened active context.

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
- `src/operation.rs`
- `src/store/tests.rs`
