# Model discovery

## Purpose

Bifrost lists all the available models based on the configured providers and
their API keys when a key is required. Windie asks Bifrost for the current
model list instead of keeping its own copy.

## Owns

Bifrost owns provider detection and model discovery. It exposes the
OpenAI-compatible `/v1/models` list and model-parameter metadata for selected
models.

Windie owns the boundary that queries Bifrost, normalizes the returned model
metadata, and exposes it to the Inspector, CLI, and runtime operations.

## Does not own

Model discovery does not configure provider keys, perform model inference, or
run conversation sessions. Provider configuration is documented in
[LLM configuration](config.md); model requests are documented in
[Model request lifecycle](lifecycle.md).

## Main flow

1. A provider is configured in Bifrost, including its API key when the
   provider requires one.
2. Windie queries Bifrost for the models currently available through those
   providers.
3. Windie normalizes the model IDs and available limits, then exposes them to
   clients and runtime operations.
4. When a client selects a model, Windie can query Bifrost for that model's
   parameters, such as reasoning or prompt-caching support.

## Important invariants

- Bifrost's current provider and model state is the source of truth. Windie
  does not maintain a duplicate provider/model table.
- Discovery is read-only with respect to conversations and session execution.
- A model appearing in the catalog means Bifrost reported it as available; it
  does not by itself start inference or change a conversation's selected
  model.

## Related code

- [`src/llm/model.rs`](../../../src/llm/model.rs) — queries Bifrost models and
  model-parameter metadata and normalizes the responses.
- [`src/llm/management.rs`](../../../src/llm/management.rs) — queries
  Bifrost's provider catalog and configured provider keys.
- [`src/operation/gateway.rs`](../../../src/operation/gateway.rs) — exposes
  shared model-list and model-parameter workflows.
- [`src/api/gateway.rs`](../../../src/api/gateway.rs) — exposes model
  discovery routes to HTTP clients.
