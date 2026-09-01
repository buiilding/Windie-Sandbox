# MCP

## Purpose

<!-- Explain MCP as the protocol boundary for discovering and calling external tools. -->

## Owns

<!-- Describe transports, tool discovery, schemas, calls, and result normalization. -->

## Does not own

<!-- Distinguish MCP transport from plugin packaging and Windie approval policy. -->

## Main flow

1. <!-- Load an installed MCP component. -->
2. <!-- Discover its tools and store the provider catalog. -->
3. <!-- Dispatch an approved call and normalize its result. -->

## Important invariants

- <!-- Only attached and available tool schemas are exposed for execution. -->
- <!-- MCP results become ordinary durable tool messages. -->

## Related code

- <!-- `src/mcp/` -->
- <!-- `src/tool/registry.rs` -->
- <!-- `src/tool/result.rs` -->
