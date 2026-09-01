# HTTP API

## Purpose

<!-- Explain the loopback HTTP boundary for Windie clients. -->

## Owns

<!-- Describe routes, request validation, authorization, and response contracts. -->

## Does not own

<!-- Distinguish the HTTP adapter from shared operations and runtime decisions. -->

## Main flow

1. <!-- Receive and authorize a client request. -->
2. <!-- Adapt it to the relevant shared operation. -->
3. <!-- Map the result or typed error back to HTTP. -->

## Important invariants

- <!-- The API remains loopback-bound and enforces paired access where required. -->
- <!-- Route handlers do not duplicate store or runtime ownership rules. -->

## Related code

- <!-- `src/api/router.rs` -->
- <!-- `src/api/state.rs` -->
- <!-- `src/api/error.rs` -->
