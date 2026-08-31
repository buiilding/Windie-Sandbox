# App connectors

## Purpose

<!-- Explain how a plugin declares access to an external application connector. -->

## Owns

<!-- Describe connector metadata, identity, authentication, and runtime projection. -->

## Does not own

<!-- Distinguish app metadata from MCP tools, skills, and Windie approval. -->

## Main flow

1. <!-- Discover connector metadata from an installed plugin. -->
2. <!-- Establish the required user connection or authorization. -->
3. <!-- Make the connected capability available through its runtime boundary. -->

## Important invariants

- <!-- Connector access remains explicit and user-controlled. -->
- <!-- Secrets and external authorization are not embedded in conversation history. -->

## Related code

- <!-- Plugin app-connector manifest and catalog modules. -->
