# Conversation trees and selected-head paths

Windie stores each conversation as one shared message tree. A model request
does not send the whole tree to the model. It selects a message head and
resolves the root-to-head path that becomes the conversation history for that
request.

This distinction is foundational:

```text
              A
              |
              B
             / \
            C   D

path to C: A, B, C
path to D: A, B, D
tree:      A, B, C, D
```

## The source of truth is the tree

Each message is persisted once with its parent message ID. Shared history is
therefore represented by shared nodes rather than copied into every branch.
The conversation tree is the canonical storage structure; a selected path is a
derived view of that structure.

This gives Windie the following properties:

- Forking adds a new branch without copying the complete history.
- Shared ancestors have one identity and one persisted copy.
- Branches can be explored without overwriting another branch.
- Message edits, truncation, deletion, and tool-group mutations operate on
  explicit tree nodes and links.
- Sessions can point at a current head without owning or copying messages.
- The database does not need to keep duplicated paths synchronized.

## What selected-head path resolution does

Given a conversation ID and a selected head message ID, the store follows the
parent links from that head toward the root, then returns the messages in
root-to-head order. This is called selected-head path resolution.

The path is what the runtime uses to construct model context. Other
conversation-level inputs, such as the system prompt and attached tool
schemas, are loaded separately because they apply to the conversation rather
than to one branch.

The path API is intentionally simple for callers:

```text
load_path_to_message(conversation_id, head_message_id)
```

Internally, the current SQLite implementation performs the parent traversal in
one recursive query. It is not making one network request per ancestor, but it
still has to resolve the ancestry because only the immediate parent is stored
on each message.

## Why Windie does not store a separate linear path for every branch

Windie could materialize paths like this:

```text
path to C: A, B, C
path to D: A, B, D
```

That would duplicate `A` and `B`. In a deep conversation with many branches,
the same history would be copied repeatedly. Any edit, deletion, truncation,
or new fork would also require updating multiple stored paths and preserving
their consistency.

Keeping parent-linked nodes normalized avoids that duplication. Path
resolution is the read-time cost paid to preserve cheap, reliable branching
and mutation semantics.

## Tree loading and path loading are different operations

The store exposes both structural row loading and complete message loading:

| Operation | Result |
| --- | --- |
| `load_message_rows` | Basic rows for every message in the conversation |
| `load_path_to_message_rows` | Basic rows only along the selected path |
| `load_messages` | Every message plus ordered parts and image data |
| `load_path_to_message` | Selected path plus ordered parts and image data |

The first pair measures tree/path structure. The second pair measures the
complete runtime message load. Benchmarks must compare the same pair of levels;
comparing complete path loading with row-only tree loading is not an
apples-to-apples comparison.

For a linear 100-message conversation, the whole tree and the selected path
contain the same 100 messages. A simple ordered scan can therefore be faster
than recursive path resolution even though it returns more general structural
data. That does not mean path resolution is always slower.

In a branched conversation, the difference is meaningful:

```text
whole tree:     1,000 messages across all branches
selected path:    100 messages on the active branch
```

The tree load still reads all 1,000 messages, while path resolution reads only
the 100 ancestors of the selected head.

## Performance policy

The tree remains the source of truth even if path resolution needs optimization.
Safe optimization directions include:

- indexes for parent and conversation lookups
- caching the selected path for an active session
- a derived ancestry index or closure table when measurements justify it
- benchmarking branched trees, not only linear chains

These techniques can improve reads without changing the ownership or mutation
model. Duplicating full linear paths should not become the default storage
model merely to optimize one access pattern.

## Architectural rule

Windie stores conversations as shared message trees. Sessions select heads in
those trees. The runtime resolves the selected root-to-head path when building
model context. The tree is canonical; the path is a derived, model-facing view.
