# Model discovery and configuration

## Purpose

<!-- Explain how Windie discovers models and configures LLM providers through Bifrost. -->

## Owns

<!-- Describe provider catalogs, credentials, model parameters, and health. -->

## Does not own

<!-- Distinguish provider setup from inference and session execution. -->

## Main flow

1. <!-- Discover supported providers or models. -->
2. <!-- Configure credentials and provider settings. -->
3. <!-- Expose available model choices to clients and runtime operations. -->

## Important invariants

- <!-- Secrets remain behind the local provider-management boundary. -->
- <!-- Model discovery does not mutate conversation execution state. -->

## Related code

- <!-- `src/llm/management.rs` -->
- <!-- `src/llm/model.rs` -->
- <!-- `src/api/gateway.rs` -->
