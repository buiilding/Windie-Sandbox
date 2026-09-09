# LLM

Windie uses one OpenAI-compatible request boundary for model inference. It
sends model requests to the local Bifrost gateway, which handles communication
with configured providers such as OpenAI, Anthropic, Ollama, and vLLM.

```text
Windie execution layer
          │ provider-neutral request
          v
    Bifrost gateway
          │ provider routing
          v
configured LLM provider
```

## Purpose

Windie is the execution layer: it owns conversation history, model-context
construction, tool execution, and durable session state. Bifrost is the
provider boundary that lets Windie use one request path without implementing
provider-specific transport inside the runtime.

## LLM roles

- **Gateway** — Bifrost's local provider-unification service and the boundary
  for provider transport.
- **Discovery** — the read-only model and model-parameter metadata reported by
  Bifrost.
- **Configuration** — provider setup and provider-key management performed
  through Bifrost's local management API.
- **Request lifecycle** — Windie's serialization, streamed-response parsing,
  and typed assistant-response assembly around a Bifrost request.

## Main flow

1. The execution layer compiles the selected conversation context and gathers
   the model, reasoning settings, and attached tool schemas.
2. Windie queries Bifrost for available models and parameters when a client
   needs discovery or configuration metadata.
3. Windie serializes the selected context into one OpenAI-compatible request
   and sends it to Bifrost.
4. Bifrost routes the request to the configured provider and streams the
   provider response back to Windie.
5. Windie parses and assembles the typed response, then the execution layer
   applies approval and tool policy and persists the runtime result.

## LLM boundaries

- Windie owns execution decisions, context selection, conversations, sessions,
  tools, approvals, and durable storage.
- Bifrost owns provider routing, provider-specific transport, provider
  configuration, model discovery, and provider response behavior.
- Model discovery and provider configuration query or change Bifrost state;
  they do not mutate conversation execution state.
- The request lifecycle translates between Windie types and Bifrost wire
  values; it does not choose context, approve tools, or persist sessions.

## References

- [Gateway](gateway.md)
- [Model request lifecycle](lifecycle.md)
- [Model discovery](discovery.md)
- [LLM configuration](config.md)
