# LLM configuration

## Purpose

For LLM configuration, Windie sends an API request to the local Bifrost
management API, and Bifrost performs the provider configuration for Windie.
Windie does not implement separate provider-specific configuration flows.

## Owns

Windie owns the local API routes that expose Bifrost's provider-management
operations to the Inspector and CLI. These routes can list providers, ensure a
provider configuration exists, create or delete a provider key, and return
redacted provider-key metadata.

Bifrost owns the provider configuration and its managed provider-key store.
Windie sends the key to Bifrost and keeps only the returned metadata needed by
the client; it does not store the LLM provider key in Windie's SQLite database.

## Does not own

LLM configuration does not own model inference, request serialization, stream
handling, conversation state, or session execution. Bifrost uses the configured
provider when Windie later sends a model request.

The Windie `~/.windie/.env` file is a separate boundary. The API writes only
secrets declared by installed tool or MCP manifests there; LLM provider keys
submitted through the management API are stored by Bifrost instead.

Model and model-parameter discovery is documented separately in
[Model discovery](discovery.md). Windie queries Bifrost for that metadata; it
does not maintain a duplicate provider/model table.

## Main flow

1. The Inspector or CLI asks a Windie API route for the provider catalog or
   selects a provider to configure.
2. Windie calls Bifrost's local management API. If needed, it first ensures
   that Bifrost has a provider configuration, then submits the provider key.
3. Bifrost stores the provider key and returns redacted status metadata. Windie
   passes that metadata back to the client.
4. Later model requests use the configured provider through Bifrost. Windie
   can query Bifrost again for the models and parameters currently available.

## Important invariants

- Provider keys remain behind Bifrost's local provider-management boundary and
  are never returned as plaintext after submission.
- Windie does not copy LLM provider keys into SQLite, conversation history, or
  its own provider table.
- Windie's `~/.windie/.env` file is restricted to manifest-declared tool/MCP
  secrets and is not the Bifrost provider-key store.
- Configuration changes do not mutate conversation or session execution state.

## Related code

- [`src/llm/management.rs`](../../../src/llm/management.rs) — Bifrost provider
  catalog, provider-key, and provider-ensure client.
- [`src/api/gateway.rs`](../../../src/api/gateway.rs) — Windie API routes that
  expose Bifrost provider management to clients.
- [`src/api/env.rs`](../../../src/api/env.rs) — separate, restricted handling
  for manifest-declared tool/MCP secrets in `~/.windie/.env`.
- [Model discovery](discovery.md) — provider models and model-parameter
  metadata queried from Bifrost.
