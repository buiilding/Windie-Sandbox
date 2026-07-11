# Tool Registry Architecture

The tool registry gives runtime one provider-neutral catalog and execution
interface. Runtime does not branch on individual MCP servers, built-in tools,
or future plugin packages.

## Unified Tool Shape

Every available tool is represented as a `ToolDefinition` containing:

- model-facing schema name;
- display name and description;
- JSON parameters;
- provider reference;
- permission lanes;
- annotations.

The provider reference contains:

- stable provider ID;
- provider-native tool name;
- provider kind.

The current provider kinds are `Mcp` and `SchemaOnly`. `SchemaOnly` identifies
a model-visible schema that deliberately has no executor.

## Registry Responsibilities

`ToolProviderRegistry` owns:

- the providers available to this Windie process;
- provider catalog loading;
- process-local successful catalog caching;
- lookup by provider ID and provider-native tool name;
- checking whether an attached tool has an executor;
- dispatching an approved call to the correct provider adapter;
- optional persistent MCP session ownership.

It does not decide approval policy and does not persist attachments. Policy
lives in `policy.rs`; conversation attachment lives in `store.rs`.

## Current Provider Sources

Today the registry is populated only from a code-approved MCP provider list:

- CUA Driver;
- Desktop Commander;
- Blender MCP;
- Bright Data;
- Exa.

There are currently no Windie-native built-in tool implementations in the
registry and no user-configured arbitrary MCP provider path.

The unified type design allows those sources to be added without changing the
runtime contract:

```text
built-in provider ---+
MCP provider --------+--> ToolDefinition --> AttachedTool --> runtime call
```

Only the MCP branch is implemented now.

## MCP Normalization

Each MCP server returns provider-native tool names and schemas. Its adapter
normalizes them by:

1. prefixing the model-facing name with a provider namespace;
2. retaining the provider-native name for dispatch;
3. copying the MCP input schema into JSON parameters;
4. assigning `ExternalProcess` permission;
5. mapping read-only annotations when supplied.

For example:

```text
provider: cua-driver
native:   click
schema:   cua_driver__click
```

Namespacing prevents two providers with a tool named `click` from colliding in
one conversation.

## Catalog Loading and Cache

Listing all available tools asks every registered provider for its catalog.
Unavailable providers are skipped by the all-tools listing. Listing one
specific provider returns its error directly.

After a provider catalog loads successfully, it is cached by provider ID for
the life of the registry. The long-running API shares one registry, so later
attachment requests reuse the cached definitions. Separate CLI processes start
with separate empty caches.

The cache is not persisted. Attached tool definitions are persisted.

## Attachment Boundary

An available definition does not grant model access. Attachment converts it to
an `AttachedTool` and stores it on one conversation.

The attached form preserves:

- the schema sent to the model;
- provider routing identity;
- permissions and annotations.

The model receives only the schema subset. If it later calls that schema name,
runtime loads the attachment and asks the registry to execute its provider
reference.

Batch attachment groups requests by provider so one provider catalog is not
reloaded for every tool. All attachments are then inserted atomically.

## Manual Schemas

Windie also supports manually inserting a raw model-facing schema. Internally
it is represented with provider ID `manual` and provider kind `SchemaOnly`.
No executor owns that reference, so it can be sent to the model for protocol
testing but policy denies execution.

## Dispatch

After policy approves a call, registry dispatch is based on provider kind:

- `Mcp`: find the approved MCP provider by ID and call its adapter;
- `SchemaOnly`: return an unknown-tool error because execution is deliberately
  unavailable.

The MCP adapter parses the model's JSON argument text, calls the
provider-native name, and normalizes text/image result blocks into one
`ToolExecutionResult`.

## Adding a New Provider Kind

A real built-in or extension implementation would need to provide the same
provider-neutral operations:

1. list `ToolDefinition` values;
2. answer whether an attachment is executable;
3. accept an attached tool plus `ToolCall`;
4. return `ToolExecutionResult`;
5. expose no provider-specific protocol details to runtime.

The registry is the ownership point for that unification. MCP JSON-RPC remains
inside `mcp.rs`; any future extension mechanism needs its own protocol boundary
rather than a speculative registry type.

## Relevant Code

- `src/tool.rs`
- `src/tool_provider.rs`
- `src/policy.rs`
- `src/operation.rs`
