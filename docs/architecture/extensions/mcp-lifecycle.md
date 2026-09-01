# MCP lifecycle

## Purpose

<!-- Explain setup, discovery, connection reuse, timeouts, and shutdown. -->

## Owns

<!-- Describe local stdio and hosted HTTP session lifecycles. -->

## Does not own

<!-- Distinguish MCP sessions from Windie conversation sessions. -->

## Main flow

1. <!-- Prepare and validate the component. -->
2. <!-- Start or connect when discovery or execution requires it. -->
3. <!-- Reuse, expire, or close the provider session. -->

## Important invariants

- <!-- Provider sessions are keyed and isolated according to their runtime identity. -->
- <!-- Timeouts produce bounded failures that the runtime can persist. -->

## Related code

- <!-- `src/mcp/mcpb.rs` -->
- <!-- `src/mcp/http.rs` -->
- <!-- MCP session coordination modules. -->
