# Gateway process

## Purpose

<!-- Explain Bifrost as an independently managed local process. -->

## Owns

<!-- Describe lifecycle, health checks, logs, and the local provider endpoint. -->

## Does not own

<!-- Distinguish process management from LLM architecture and Windie state. -->

## Main flow

1. <!-- Resolve and start the managed gateway process. -->
2. <!-- Report readiness and accept provider requests. -->
3. <!-- Stop independently without changing Windie conversations or sessions. -->

## Important invariants

- <!-- Gateway lifecycle remains independent from the API lifecycle. -->
- <!-- Process state is observed explicitly through health and PID records. -->

## Related code

- <!-- `src/llm/gateway.rs` -->
- <!-- `src/local/process.rs` -->
- <!-- `src/operation/system.rs` -->
