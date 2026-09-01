# Model access

Windie uses one OpenAI-compatible request boundary for model inference. It
sends model requests to the local Bifrost gateway, which handles communication
with configured providers such as OpenAI, Anthropic, Ollama, and vLLM.

Windie owns conversation history, model-context construction, tool execution,
and durable session state. Bifrost owns provider routing, provider-specific
transport, model discovery, and provider configuration.

## References

- [Bifrost gateway](bifrost-gateway.md)
- [Model request lifecycle](model-request-lifecycle.md)
- [Model discovery and configuration](model-discovery-and-configuration.md)
