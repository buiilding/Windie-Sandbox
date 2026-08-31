# Tool execution

## Purpose

Tool execution turns a model-requested tool call into a durable result in the
conversation. It gives the model useful capabilities while keeping the
decision to execute explicit, inspectable, and controlled by the conversation
approval mode.

Windie supports provider-backed tools, such as MCP tools, and Windie-owned
built-in tools such as reading a plugin skill or attaching a plugin MCP.

## Owns

The tool-execution path finds the next pending tool call, resolves it to an
attached tool definition, applies policy, dispatches an allowed call to the
correct provider or built-in implementation, and saves the resulting tool
message.

## Does not own

The model decides which available tool to request; tool execution does not
invent tool calls. The runtime turn owns the wider model → tool → model loop,
while the conversation tree owns the assistant message and tool-result
messages that make the result durable.

Provider installation, health, credentials, and component setup are separate
concerns. A tool must have an available registered provider before the policy
can allow it to run.

## Main flow

1. The model returns an assistant message that may contain one or more tool
   calls. Windie saves that assistant message before resolving any calls.
2. Windie selects the next unresolved call in the order the model requested
   them and finds its tool schema. Provider-backed tools must be attached to
   the conversation; Windie built-in tools are supplied by the runtime.
3. Windie checks that the tool is executable. A detached tool, unavailable
   provider, or missing executor is denied and produces a failed tool result.
4. For an executable tool, the conversation's approval mode decides whether
   to ask or execute:
   - `Manual` pauses the session at `WaitingForApproval` and asks the user to
     approve or deny that specific call.
   - `AutoApproveAttached` executes an attached, available tool immediately.
     It does not grant access to detached tools or unavailable providers.
5. Windie executes an allowed call through either a built-in implementation or
   the registered provider, such as an MCP component. It saves the success or
   failure as a tool message linked to the assistant's tool-call ID.
6. The runtime continues with the next pending call, or rebuilds model context
   and lets the model respond to the saved tool result.

```text
assistant message with tool calls
             │
             v
select next unresolved call in order
             │
             v
attached and executable?
    ├── no ───────────────> save denied tool result
    │
    └── yes
          │
          ├── Manual ───────────────> wait for approval
          │                              ├── deny ──> save denied result
          │                              └── approve ─┐
          │                                           │
          └── AutoApproveAttached ────────────────────┤
                                                      v
                                   execute built-in or provider tool
                                                      │
                                                      v
                                    save tool result and continue
```

## Important invariants

- Approval mode is stored on the conversation and defaults to `Manual` for a
  new conversation.
- `AutoApproveAttached` is not unrestricted execution. It only auto-approves
  tools that are already exposed to the conversation and backed by an available
  registered provider.
- In `Manual` mode, every model-requested executable tool call requires a
  decision, including Windie built-in tools.
- Tool calls from one assistant message are resolved in order. A later call
  cannot run while an earlier call remains unresolved.
- Every resolved call produces a durable tool result linked to the original
  tool-call ID. A denial, provider error, or timeout is still a result the
  model can read on the next turn.
- Tool schemas attached to a conversation are shared by its branches. The
  selected message head controls message history, not which attached provider
  tools are available.

## Related code

- `src/tool/policy/mod.rs`: `Allow`, `Ask`, and `Deny` decisions.
- `src/tool/approval.rs`: conversation-level approval modes.
- `src/runtime/tool_execution.rs`: pending-call order, tool lookup, dispatch,
  and result persistence.
- `src/runtime/turn.rs`: tool resolution within the wider runtime loop.
- `src/tool/registry.rs`: registered provider lookup and tool execution.
- `src/mcp/result.rs`: MCP failures and timeouts converted into tool results.
