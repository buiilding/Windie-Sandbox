# Bifrost gateway

## Purpose

Windie is more like an execution layer for AI on the operating system. I do
not want to spend time designing and maintaining provider-specific gateway
code inside Windie, so Windie uses Bifrost as its LLM gateway.

Bifrost is advertised as a lightweight gateway, which is the reason for using
it here. Windie sends Bifrost one provider-neutral request shape, and Bifrost
handles the differences between LLM providers. The local gateway listens on
`http://localhost:8080` by default.

## Owns

Bifrost owns the provider boundary:

- routing requests to the configured LLM provider;
- provider-specific HTTP transport and response behavior;
- provider configuration and provider-key management;
- model discovery, model-parameter metadata, and input-token counting; and
- the OpenAI-compatible model and streaming response endpoints used by Windie.

Windie owns the thin adapter around that boundary. It serializes Windie
messages, reasoning settings, and tool schemas into Bifrost's request shape,
then parses the streamed response into Windie runtime events.

## Does not own

Bifrost does not own Windie's execution layer, conversations, sessions,
messages, context selection, tool approval, tool execution, or SQLite storage.
It returns provider results; Windie decides what to do with those results and
persists the runtime state.

Bifrost also does not decide Windie's process lifecycle. Windie can start,
stop, and check the local gateway independently from the API and other local
components.

## Main flow

1. The Windie runtime compiles the selected context and prepares the model,
   reasoning settings, and attached tool schemas.
2. The LLM adapter serializes that data into an OpenAI-compatible request and
   sends it to Bifrost.
3. Bifrost routes the request to the configured provider and streams the
   provider output back to Windie.
4. Windie parses the stream into reasoning, assistant-text, and tool-call
   events. The execution layer applies approval policy, executes allowed tools,
   and persists the completed runtime result.

## Important invariants

- Windie uses one provider-neutral request path through Bifrost instead of
  implementing a separate transport for each provider.
- Bifrost is the provider boundary, not the authority for Windie conversations,
  sessions, tools, or durable runtime state.
- A healthy Bifrost process does not guarantee that a provider, model, or API
  key is configured and ready for inference.
- Provider-specific details stay behind the Bifrost and LLM adapter boundary;
  the execution layer consumes Windie-typed results.

## Related code

- [`src/llm/client.rs`](../../../src/llm/client.rs) — sends OpenAI-compatible
  model and input-token requests to Bifrost.
- [`src/llm/serialization.rs`](../../../src/llm/serialization.rs) — converts
  Windie messages, tools, and settings into provider request data.
- [`src/llm/stream.rs`](../../../src/llm/stream.rs) — parses streamed provider
  output and assembles the typed assistant response.
- [`src/llm/gateway.rs`](../../../src/llm/gateway.rs) — checks Bifrost health
  and manages its local process lifecycle.
- [`src/llm/management.rs`](../../../src/llm/management.rs) — calls Bifrost's
  provider-management API.
