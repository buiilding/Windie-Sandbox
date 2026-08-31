# Bifrost gateway

## Purpose

<!-- Explain why Windie uses Bifrost as its provider-unification boundary. -->

## Owns

<!-- Describe model routing, provider transport, discovery, and configuration. -->

## Does not own

<!-- Distinguish Bifrost from Windie conversations, sessions, tools, and storage. -->

## Main flow

1. <!-- Windie prepares an OpenAI-compatible request. -->
2. <!-- Bifrost routes it to the configured provider. -->
3. <!-- Streamed provider output returns to Windie. -->

## Important invariants

- <!-- Windie uses one provider-neutral request path. -->
- <!-- Bifrost does not become the authority for Windie runtime state. -->

## Related code

- <!-- `src/llm/client.rs` -->
- <!-- `src/llm/gateway.rs` -->
- <!-- `src/llm/management.rs` -->
