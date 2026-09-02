# Context compiler

## Purpose

Before querying the gateway, Windie needs to build the context that will be
queried. The context compiler gets the selected conversation path from the
root to the current head, loads the conversation's system prompt and attached
tool schemas, and adds the current plugin index and Windie-owned tool schemas.
The result is the exact read-only message and tool-schema payload sent to the
model.

## Owns

- Resolving the selected root-to-head message path from the conversation tree.
  The path includes the interaction messages the model should see for that
  branch, including their text, images, roles, and metadata.
- Loading the conversation-wide user-owned system prompt.
- Loading the latest compaction summary when its checkpoint is on the selected
  path. Messages at or before that checkpoint are replaced by the summary, and
  messages after it remain in the path.
- Loading the conversation's attached tool schemas.
- Generating the current plugin index from the installed plugin catalog. The
  plugin index is a separate generated system message; it is not stored inside
  or merged into the user's system-prompt value.
- Appending Windie-owned built-in tool schemas when they do not duplicate an
  attached schema.
- Returning one `ModelContext` containing the final messages and tool schemas.

The final message order is:

1. the generated plugin-index system message, when a plugin catalog is
   available;
2. the user-owned system prompt, when one is set;
3. the applicable compaction summary, when one is on the selected path; and
4. the selected root-to-head path, or the part after the compaction checkpoint.

## Does not own

- The conversation tree or system-prompt storage. The store remains the source
  of truth for those inputs.
- Choosing the model, reasoning mode, or provider.
- Serializing the context into the provider's wire format. The LLM
  serialization boundary handles that after compilation.
- Querying the gateway or executing tools.
- Persisting the generated plugin index or built-in schemas as conversation
  messages.

## Main flow

1. Windie receives a conversation ID and an explicit selected message head.
2. The compiler resolves the durable root-to-head path and loads the
   conversation-wide system prompt, latest compaction, and attached schemas.
3. It applies the compaction checkpoint if it belongs to the selected path.
4. It generates the plugin index and adds the built-in control schemas for the
   current runtime.
5. It returns the exact messages and tool schemas for the request. Execution,
   inspection, and input-token counting use this same compiled context.

## Important invariants

- Every model request receives freshly compiled context for its selected head.
- The selected head determines the branch history the model sees; the whole
  conversation tree is not sent to the model.
- The user-owned system prompt and attached tool schemas apply across the
  conversation's branches.
- The plugin index and built-in schemas are generated runtime metadata. They
  are model-visible but are not saved as user-owned conversation state.
- A compaction checkpoint only applies when its checkpoint message is on the
  selected path. A checkpoint from another branch is ignored.
- Context compilation is read-only. It does not mutate the conversation or
  invoke the gateway.

## Related code

- [`src/runtime/context.rs`](../../../src/runtime/context.rs) — exact
  model-context construction and ordering.
- [`src/operation/inspection.rs`](../../../src/operation/inspection.rs) —
  exposes the compiled context and schemas for developer inspection.
- [`src/llm/serialization.rs`](../../../src/llm/serialization.rs) — converts
  the compiled Windie context into the provider request format.
- [`src/runtime/turn.rs`](../../../src/runtime/turn.rs) — uses the compiled
  context for model execution.
- [`docs/architecture/storage/conversation-tree-paths.md`](../storage/conversation-tree-paths.md)
  — explains selected-head path resolution.
