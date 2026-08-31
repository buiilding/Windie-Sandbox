# Context compiler

## Purpose

<!-- Explain how Windie builds the exact messages and tool schemas sent to a model. -->

## Owns

<!-- Describe selected-path loading, prompts, compaction, tools, and generated metadata. -->

## Does not own

<!-- Distinguish context compilation from persistence and model execution. -->

## Main flow

1. <!-- Load the selected durable state. -->
2. <!-- Combine user-owned and runtime-generated context in order. -->
3. <!-- Return the final read-only model payload. -->

## Important invariants

- <!-- Every model request receives freshly compiled context. -->
- <!-- Generated runtime metadata is not saved as user-owned conversation state. -->

## Related code

- <!-- `src/runtime/context.rs` -->
- <!-- `src/operation/inspection.rs` -->
- <!-- `src/llm/serialization.rs` -->
