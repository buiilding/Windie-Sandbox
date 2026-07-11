# Tool Execution Architecture

Tool execution connects an assistant-requested function call to one attached
tool, one policy decision, one provider execution, and one durable tool output.

## Tool Visibility Before Execution

A model can request a tool only when its schema was attached to the
conversation and sent with the Bifrost request. Attachment is both a
model-visibility boundary and the first execution boundary.

An attached tool stores:

- model-facing schema name, description, and JSON parameters;
- provider ID and provider-native tool name;
- provider kind;
- permission and annotation metadata.

The model calls the model-facing schema name. Runtime uses that name to load the
conversation attachment and recover the provider mapping.

## Assistant Tool Calls

One assistant message can own several calls in its metadata:

```text
assistant node
  metadata.tool_calls = [call_1, call_2, call_3]
```

Each call contains:

- stable provider call ID;
- metadata order index;
- kind, currently `function`;
- function name;
- raw JSON argument text.

There is one assistant node for the turn, not one node per call.

## Tool Outputs

Every resolved call receives its own `role: tool` message. Outputs form a
linear chain in assistant metadata order:

```text
assistant [call_1, call_2, call_3]
        |
tool output for call_1
        |
tool output for call_2
        |
tool output for call_3
```

The first output is parented by the assistant. Each later output is parented by
the previous output. Every output metadata object contains the one
`tool_call_id` it answers.

When context is serialized, assistant calls become separate Responses
`function_call` items and tool nodes become matching `function_call_output`
items.

## Why Calls Resolve in Order

Runtime finds the latest assistant tool-call node on the active path and scans
the contiguous tool nodes after it. The first requested ID without a result is
the only call that may resolve next.

Trying to approve call 2 while call 1 is unresolved is rejected. The ordering
keeps one deterministic parent chain and a valid provider request history.

Another model query is blocked until every requested call has an output.

## Policy Inputs

Policy receives:

- the requested tool call;
- the matching conversation attachment, if one exists;
- whether the current registry has an executor for that attachment;
- the conversation's approval mode.

It returns a typed decision:

| Condition | Decision |
| --- | --- |
| Tool is not attached | Deny |
| Attachment has no registered executor | Deny |
| Executable attachment in manual mode | Ask |
| Executable attachment in auto-approve mode | Allow |

Current permission metadata does not further restrict an attached executable
tool in auto-approve mode. Read-only hints are retained for UI/metadata, but the
policy does not yet use them to distinguish safe and risky calls.

## Manual Approval

In `manual` mode, runtime stops at the first executable pending call and exposes
one approval request containing:

- the assistant message ID;
- the complete tool call;
- the reason approval is required.

Approving reevaluates the attachment and executor, invokes the provider, and
stores the output. Denying does not invoke the provider; it stores an
error-shaped output stating that the user rejected the call.

The API's approval and denial runs continue automatically if no later manual
approval remains. The one-shot CLI `approve` and `deny` commands store one
result and exit; the user invokes `query` explicitly afterward.

## Automatic Execution

In `auto_approve_attached` mode, runtime executes attached tools that the
registry can handle. It stores each result and inspects the next call. Once all
outputs exist, runtime queries the model again.

The automatic loop is bounded by model responses and tool policy:

```text
pending call
   |
   +-- Deny  -> store failed output -> inspect next
   +-- Allow -> execute/store       -> inspect next
   +-- Ask   -> stop for user
   +-- none  -> query model
```

This is not unrestricted autonomous execution. A manual decision always stops
progression.

## Denied and Failed Outputs

Provider protocols require an output for every function call. Windie therefore
persists failures as normal linked tool messages.

Failure examples include:

- unattached tool;
- unknown executor;
- explicit user denial;
- MCP timeout;
- MCP process or protocol failure;
- malformed provider result processing.

An approved MCP execution failure becomes a structured model-facing result
rather than aborting the whole call contract. Failures before execution, such
as provider catalog startup errors, remain operation errors because no
assistant call is waiting for an output.

## Rich Outputs

Tool results contain a compact visible `content` preview and optional ordered
text/image parts. Text-only results can store just `content`. Screenshot-like
MCP results persist typed image parts so later model requests do not need to
carry base64 as visible text.

## Store Validation

The store accepts a tool output only when:

- its parent belongs to the same conversation;
- the parent is the assistant call node or an output in its chain;
- walking through tool parents reaches the owning assistant;
- the assistant requested the supplied call ID.

Generic message insertion cannot create `role: tool` nodes.

## Group Deletion

The tool-calling assistant and all tool outputs below it are one deletion unit.
Deleting the assistant or any output removes the entire group, then reparents
surviving descendants to the assistant's parent.

```text
user A -> assistant calls -> output 1 -> output 2 -> assistant B

delete any call/output node

user A -> assistant B
```

This prevents dangling calls and outputs. Editing is currently less strict:
visible text can be changed while metadata links remain.

## Relevant Code

- `src/tool.rs`
- `src/policy.rs`
- `src/runtime.rs`
- `src/store.rs`
- `src/tool_provider.rs`
- `src/runtime_tests.rs`
- `src/policy_tests.rs`
