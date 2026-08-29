# Runtime turn

## Purpose

A runtime turn is one execution cycle for a session. It takes the current
conversation branch forward from an input or wakeup until the assistant can
finish, needs approval, fails, or is cancelled.

Normal user input is the common trigger. The same runtime path also supports
manual and idle wakeups, and is intended to support future scheduled, file,
browser, and system-event wakeups.

## Owns

The runtime owns the execution loop: building the exact model request,
streaming and saving assistant responses, resolving tool calls in order, and
stopping at a clear outcome.

## Does not own

The runtime does not own the conversation tree, the session lifecycle, or the
LLM gateway process. The store owns durable conversation and session state;
the session manager owns claims, queues, cancellation, and wakeup scheduling;
and Bifrost routes Windie's model request to the configured provider.

The runtime also does not decide whether a tool is safe without help. Tool
policy, provider state, and the conversation's approval mode determine whether
a requested tool is allowed, denied, or shown to the user for approval.

## Main flow

1. A session receives input or a wakeup and selects the branch head from which
   execution will continue. The input is part of the durable conversation path;
   the runtime does not keep a separate mutable transcript in memory.
2. Windie builds a fresh model context for that head. It loads the selected
   root-to-head message path, the conversation system prompt, any compaction
   summary, attached tool schemas, and ephemeral runtime metadata such as the
   current plugin index and built-in control schemas.
3. Windie sends that context to Bifrost, which forwards it to the configured
   model provider. Assistant text and tool-call metadata stream back, then
   Windie saves the completed assistant message under the current head.
4. If the assistant did not request a tool, the runtime turn is complete.
5. If it did request tools, Windie resolves pending calls one at a time. For
   each call it finds the attached tool, checks provider state and policy, then
   either saves a denied result, pauses for user approval, or executes the
   tool. An executable call is dispatched to a Windie built-in tool or the
   relevant provider, such as an MCP component.
6. Windie saves every tool result as a durable tool message. It then rebuilds
   context from the new head and queries the model again. The cycle continues
   until there are no further tool calls or execution reaches another outcome.

```text
input or wakeup
      │
      v
build model context from the selected conversation path
      │
      v
query model through Bifrost → save assistant message
      │
      ├── no tool calls ───────────────> completed
      │
      └── tool calls ─> policy and approval check
                              ├── waiting for approval ──> paused
                              ├── denied ────────────────> save tool result
                              └── allowed ─> execute tool ─> save tool result
                                                            │
                                                            v
                                             rebuild context and continue
```

## Important invariants

- Model context is rebuilt for every model request. It is a projection of
  durable state, not a transcript that the runtime mutates in place.
- The selected message head determines the branch history the model sees.
  Saving an assistant response or tool result advances that head.
- An assistant message is saved before its tool calls are resolved, and each
  resulting tool message is saved before the next model request. This makes
  the complete execution path durable and inspectable.
- Tool calls are resolved in their requested order. A later call cannot bypass
  an earlier unresolved call.
- A runtime turn completes only after the most recent assistant response has
  no pending tool calls. It can instead pause for approval, fail, or be
  cancelled.

## Related code

- `src/runtime/turn.rs`: context-to-model-to-tool execution loop.
- `src/runtime/context.rs`: exact model-context compiler for one selected
  conversation head.
- `src/runtime/tool_execution.rs`: tool lookup, policy decisions, dispatch,
  and durable tool-result handling.
- `src/runtime/wakeup.rs`: typed wakeup inputs and their prompts.
- `src/operation/session.rs`: session-level runtime entry point and Bifrost
  request setup.
- `src/session/manager.rs`: durable session execution, claims, queues, and
  background wakeup scheduling.
