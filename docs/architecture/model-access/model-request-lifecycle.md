# Model request lifecycle

## Purpose

<!-- Explain the request and streaming path from compiled context to saved response. -->

## Owns

<!-- Describe serialization, HTTP transport, stream parsing, and response assembly. -->

## Does not own

<!-- Distinguish provider transport from context compilation and runtime decisions. -->

## Main flow

1. <!-- Serialize Windie messages, tools, and model parameters. -->
2. <!-- Send the request through Bifrost and consume stream events. -->
3. <!-- Assemble the typed response returned to the runtime. -->

## Important invariants

- <!-- Provider-specific details remain behind the gateway boundary. -->
- <!-- Partial streaming presentation does not replace the durable final response. -->

## Related code

- <!-- `src/llm/serialization.rs` -->
- <!-- `src/llm/client.rs` -->
- <!-- `src/llm/stream.rs` -->
