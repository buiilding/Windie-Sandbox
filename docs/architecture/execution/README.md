# Execution

Execution begins when a wakeup activates a session. Windie selects the
session's current conversation head, compiles the exact model context, queries
the model, saves its response, and resolves requested tools in order. The loop
ends when the model finishes, execution needs approval, or the session fails or
is cancelled.

```text
wakeup
  │
  v
context compiler → model request → save assistant response
                                      │
                                      ├── no tools → complete
                                      └── tools → policy and approval
                                                       │
                                                       v
                                              save result and continue
```

## References

- [Wakeups](wakeups.md)
- [Context compiler](context-compiler.md)
- [Runtime loop](runtime-loop.md)
- [Tool execution](tool-execution.md)
- [Approvals](approvals.md)
