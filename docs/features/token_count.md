# Conversation Token Counting

Windie can ask Bifrost for the input-token cost of the context that would be
sent on the next query. Counting is read-only and separate from inference.

## What Windie Counts

The count input is built from the same sources as a real model request:

- the selected root-to-active message path;
- conversation system prompt;
- applicable compaction summary;
- ordered text and image message parts;
- assistant tool calls and tool outputs on the path;
- every attached tool schema;
- the resolved model.

The full conversation tree is not counted. Inactive branches are not part of
model context.

## Count Request Flow

```text
inspector requests count
        |
        v
API resolves persisted model or request override
        |
        v
ContextBuilder flattens current model context
        |
        v
store loads attached tool schemas
        |
        v
API drops its SQLite store before awaiting network I/O
        |
        v
BifrostClient calls /responses/input_tokens
        |
        v
API returns count, unavailable, or empty-context result
```

The request does not run query preparation because preparation may persist
policy-denied tool outputs. Counting must not mutate the conversation.

## Empty Context

When both messages and attached tools are empty, there is nothing to count.
Windie returns `EmptyContext` without requiring the gateway or making an HTTP
request.

The API response has null count, model, source, and raw fields.

## Tool-Only Context

Bifrost's Responses input-token endpoint requires at least one input item even
when the interesting cost is entirely in tool schemas.

For a conversation with tools but no messages, Windie adds a tiny synthetic
system input only to the counting request. It is never:

- persisted;
- shown in the tree;
- added to a real model query.

The response source is `prequery_synthetic_input` so clients can distinguish
this workaround from a normal count.

## Bifrost Request Shape

The count endpoint receives:

- selected model;
- serialized Responses input items;
- serialized function tools.

Windie's normal message serializer is reused, including assistant function
calls, linked function outputs, and image parts. This keeps token preview close
to real request shape.

The typed successful response contains:

- `input_tokens`;
- optional `total_tokens`;
- optional measured model identity;
- complete raw Bifrost/provider response.

## Unsupported Providers

Not every provider path supports pre-query input counting. The LLM boundary
recognizes Bifrost's unsupported input-token response and returns a typed
`Unsupported` outcome.

The API converts it to a successful unavailable response:

```json
{
  "input_tokens": null,
  "total_tokens": null,
  "model": "provider/model",
  "source": "unavailable",
  "raw": null
}
```

Network failures, malformed responses, and other provider errors remain errors.
Only the known capability gap becomes `Unsupported`.

## When the Inspector Counts

The backend does not monitor SQLite or update a stored counter. The inspector
requests a count whenever it loads full conversation state unless that load
explicitly disables counting.

Most mutations use this sequence:

```text
mutation -> reload conversation -> count current input
```

This means path activation, editing, inserting, removal, truncation, forking,
system-prompt changes, model changes, and tool attachment changes normally lead
to a new count through the reload path.

During a durable run, `assistant_message_saved` and `tool_result_saved` events
also reload and count. The final `query_done` reload disables another count to
avoid immediately repeating the count already triggered by the persisted
message event.

## Context Signatures

The inspector creates a stable JSON signature for the model-facing state. It
includes:

- path node IDs and roles;
- message parts;
- tool-call arrays and result call IDs;
- conversation ID and model;
- system prompt;
- attached schema names, descriptions, parameters, and provider mapping;
- latest compaction.

Each asynchronous request also gets a local request ID. A response updates UI
state only when its ID is still the newest request for that conversation/model
key. This prevents an older slow count from replacing a newer result.

The signature records what the count measured. It is not sent to or persisted
by the backend.

## Post-Query Fallback

Completed assistant metadata can contain provider-reported usage:

- input tokens;
- output tokens;
- total tokens;
- raw provider usage.

When the active path ends in an assistant with total usage, the inspector can
immediately use that total as `postquery_total`. It is a display fallback while
an exact current pre-query count is unavailable or pending.

This fallback is not equivalent to a fresh input count:

- it is total usage rather than necessarily input-only usage;
- it describes the completed query;
- later prompt, path, message, model, or tool changes may make it stale.

An exact `prequery_input` result for the current signature takes precedence.
If explicit counting later reports unavailable, the inspector preserves an
existing usable fallback instead of replacing the meter with nothing.

## Token Meter Maximum

The inspector compares the chosen current value with the selected catalog
model's `context_length`, falling back to `max_input_tokens`. Those limits come
from Bifrost's model list, not from the count response.

## What Is Persisted

The pre-query count itself is not persisted in SQLite. It lives in inspector
state keyed by conversation and model.

Provider usage attached to a completed assistant message is durable because it
is part of that message's metadata.

## Relevant Code

- `src/context.rs`
- `src/operation.rs`
- `src/llm.rs`
- `src/api.rs`
- `dev/windie-inspector/src/lib/windieApi.js`
- `dev/windie-inspector/src/context/WindieContext.jsx`
