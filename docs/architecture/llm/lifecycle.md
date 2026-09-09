# Model request lifecycle

## Purpose

Windie receives the compiled model context from the execution layer, sends it
to Bifrost, and receives a streamed response. This document explains how that
request becomes a complete Windie response.

Windie does not create the provider stream itself. Bifrost returns the
OpenAI-compatible streamed response to Windie; Windie parses that stream and
can then publish its own runtime events to API clients.

## Owns

The model request boundary owns:

- serializing Windie messages, images, reasoning settings, prompt-cache hints,
  and tool schemas into the OpenAI-compatible Responses request;
- sending the request to Bifrost's `/responses` endpoint with streaming
  enabled;
- reading provider bytes safely across network chunks and SSE lines;
- parsing streamed provider events into assistant-text, reasoning, and tool-call
  deltas; and
- assembling the complete assistant content, metadata, tool calls, usage, and
  finish reason returned to the runtime.

## Does not own

This boundary does not compile the conversation context, choose the selected
message head, decide whether a tool call is allowed, execute tools, or persist
messages and session state. Those responsibilities belong to the execution,
approval, and storage layers.

It also does not own provider routing or provider configuration. Bifrost is the
provider gateway, while Windie owns the provider-neutral request and response
types around it.

## Main flow

1. The runtime compiles the selected conversation path, system prompt, model,
   reasoning settings, and attached tool schemas.
2. Windie serializes messages and tools into the OpenAI-compatible Responses
   request. Images are encoded as provider-compatible image blocks, and
   provider-specific prompt-cache fields are added only when the model allows
   them.
3. Windie sends the request to Bifrost with `stream: true`. Bifrost routes it
   to the configured provider and returns streamed `data:` events.
4. Windie buffers network bytes until valid UTF-8 and complete SSE lines are
   available. It parses each JSON event, forwards display deltas to the runtime
   output, and accumulates text, reasoning, refusal data, tool-call arguments,
   and usage metadata.
5. When the stream ends, Windie validates and assembles an `AssistantResponse`.
   The runtime uses that complete response to save the assistant message and,
   when tool calls are present, continue through its approval and execution
   flow.

## Important invariants

- Provider-specific transport and routing remain behind Bifrost; Windie uses
  one provider-neutral Responses request and typed response boundary.
- Network chunks may split UTF-8 characters or SSE lines, so parsing waits for
  complete valid input before decoding an event.
- Partial text, reasoning, and tool-call deltas are transient updates. The
  assembled `AssistantResponse` is the source of truth for runtime persistence.
- Tool calls are not executed from partial arguments. They are assembled and
  validated before the runtime applies approval policy and executes them.
- Provider stream errors, malformed events, invalid UTF-8, or an empty final
  response fail the request instead of creating a false completed assistant
  message.

## Related code

- [`src/llm/serialization.rs`](../../../src/llm/serialization.rs) — converts
  Windie messages, images, tools, reasoning, and cache hints into Responses
  request data.
- [`src/llm/client.rs`](../../../src/llm/client.rs) — sends the request to
  Bifrost and reads its streamed HTTP response.
- [`src/llm/responses.rs`](../../../src/llm/responses.rs) — defines the
  OpenAI-compatible request and streamed-event wire shapes.
- [`src/llm/stream.rs`](../../../src/llm/stream.rs) — parses SSE lines and
  assembles the typed assistant response.
- [`src/runtime/turn.rs`](../../../src/runtime/turn.rs) — supplies runtime
  execution with the completed model response.
