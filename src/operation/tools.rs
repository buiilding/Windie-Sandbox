//! Tool catalog, attachment, schema, and approval operations.

use super::{
    ConversationId, HashMap, ProviderToolName, Result, Store, ToolApprovalMode,
    ToolApprovalRequest, ToolDefinition, ToolProviderId, ToolProviderRegistry, ToolSchema,
    ToolSchemaName, error, pending_tool_approvals, pending_tool_approvals_with_registry,
};

pub fn set_tool_approval_mode(
    store: &mut Store,
    conversation_id: &ConversationId,
    mode: ToolApprovalMode,
) -> Result<()> {
    store.set_tool_approval_mode(conversation_id, mode)
}

/// Lists provider tools that can be attached to conversations.
pub fn available_tools() -> Result<Vec<ToolDefinition>> {
    let registry = ToolProviderRegistry::new();

    available_tools_with_registry(&registry)
}

/// Lists provider tools through a caller-owned registry.
pub fn available_tools_with_registry(
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolDefinition>> {
    registry.list_available_tools()
}

/// Lists provider tools for one provider ID.
pub fn available_provider_tools(provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
    let registry = ToolProviderRegistry::new();

    available_provider_tools_with_registry(&registry, provider_id)
}

/// Lists one provider's tools through a caller-owned registry.
pub fn available_provider_tools_with_registry(
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<Vec<ToolDefinition>> {
    registry.list_provider_tools(provider_id)
}

/// Attaches one available provider tool to a conversation.
pub fn attach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
) -> Result<ToolSchemaName> {
    let registry = ToolProviderRegistry::new();

    attach_tool_with_registry(store, conversation_id, provider_id, tool_name, &registry)
}

/// Attaches one available provider tool using a caller-owned registry.
pub fn attach_tool_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
    registry: &ToolProviderRegistry,
) -> Result<ToolSchemaName> {
    let definition = registry.find_tool(provider_id, tool_name)?.ok_or_else(|| {
        error::not_found(format!("tool does not exist: {provider_id}/{tool_name}"))
    })?;
    let attached_tool = definition.attached_tool();
    let schema_name = attached_tool.schema_name.clone();

    store.insert_attached_tool(conversation_id, &attached_tool)?;

    Ok(schema_name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One requested provider tool attachment in a batch operation.
pub struct ToolAttachmentInput {
    pub provider_id: ToolProviderId,
    pub tool_name: ProviderToolName,
}

impl ToolAttachmentInput {
    /// Builds a typed attachment request from provider identity parts.
    pub fn new(provider_id: ToolProviderId, tool_name: ProviderToolName) -> Self {
        Self {
            provider_id,
            tool_name,
        }
    }
}

/// Attaches multiple available provider tools using a caller-owned registry.
///
/// The provider catalog is loaded at most once per provider ID in the request,
/// so provider-level UI actions do not restart an MCP server for each tool.
/// Storage remains strict: duplicate schema names fail the batch insert.
pub fn attach_tools_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    requests: &[ToolAttachmentInput],
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolSchemaName>> {
    let mut provider_catalogs: HashMap<ToolProviderId, HashMap<ProviderToolName, ToolDefinition>> =
        HashMap::new();

    for request in requests {
        if provider_catalogs.contains_key(&request.provider_id) {
            continue;
        }
        let provider_tools = registry.list_provider_tools(&request.provider_id)?;
        provider_catalogs.insert(
            request.provider_id.clone(),
            provider_tools
                .into_iter()
                .map(|definition| (definition.provider.tool_name.clone(), definition))
                .collect(),
        );
    }

    let mut attached_tools = Vec::with_capacity(requests.len());
    let mut schema_names = Vec::with_capacity(requests.len());
    for request in requests {
        let definition = provider_catalogs
            .get(&request.provider_id)
            .and_then(|provider_tools| provider_tools.get(&request.tool_name))
            .ok_or_else(|| {
                error::not_found(format!(
                    "tool does not exist: {}/{}",
                    request.provider_id, request.tool_name
                ))
            })?;
        let attached_tool = definition.attached_tool();
        schema_names.push(attached_tool.schema_name.clone());
        attached_tools.push(attached_tool);
    }

    store.insert_attached_tools(conversation_id, &attached_tools)?;

    Ok(schema_names)
}

/// Inserts one conversation-level tool schema.
pub fn insert_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.insert_tool_schema(conversation_id, tool_schema)
}

/// Updates one conversation-level tool schema.
pub fn update_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    current_name: &ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.update_tool_schema(conversation_id, current_name, tool_schema)
}

/// Removes one conversation-level tool schema.
pub fn remove_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    name: &ToolSchemaName,
) -> Result<()> {
    store.remove_tool_schema(conversation_id, name)
}

/// Detaches one model-facing tool schema from a conversation.
pub fn detach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    schema_name: &ToolSchemaName,
) -> Result<()> {
    remove_tool_schema(store, conversation_id, schema_name)
}

/// Lists pending active-path tool calls that need user approval.
pub fn list_tool_approvals(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<Vec<ToolApprovalRequest>> {
    pending_tool_approvals(store, conversation_id)
}

/// Lists pending active-path tool calls through a caller-owned registry.
pub fn list_tool_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolApprovalRequest>> {
    pending_tool_approvals_with_registry(store, conversation_id, registry)
}
