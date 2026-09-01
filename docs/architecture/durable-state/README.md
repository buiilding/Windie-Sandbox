# Durable state

Durable state is the part of Windie that survives a client disconnect or local
process restart. SQLite stores the conversation tree, session records, queued
inputs, approvals, execution claims, and replayable events.

```text
conversation tree: durable message history
        │
        └── selected head ← session current position and lifecycle
                                  │
                                  ├── execution claim protects writes
                                  └── durable events record activity

                  all persisted in SQLite
```

The conversation tree answers what was said. A session answers where execution
is operating and whether it can continue. Execution claims prevent an older
runner from writing after ownership changes. Durable events let clients replay
and follow session activity without making the event stream the source of
conversation truth.

## References

- [Conversation tree](conversation-tree.md)
- [Conversation tree paths](conversation-tree-paths.md)
- [Sessions](sessions.md)
- [Session lifecycle](session-lifecycle.md)
- [Execution claims](execution-claims.md)
- [SQLite storage](sqlite-storage.md)
- [Durable events](durable-events.md)
