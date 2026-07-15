//! Tool catalog, attachment, and tool-schema operation workflows.

use super::*;

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

/// Attaches one available provider tool to an explicit conversation path.
pub fn attach_tool_with_registry_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
    registry: &ToolProviderRegistry,
) -> Result<ToolSchemaName> {
    let definition = registry.find_tool(provider_id, tool_name)?.ok_or_else(|| {
        error::not_found(format!("tool does not exist: {provider_id}/{tool_name}"))
    })?;
    let attached_tool = definition.attached_tool();
    let schema_name = attached_tool.schema_name.clone();

    store.insert_attached_tool_at_head(conversation_id, head_message_id, &attached_tool)?;

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

/// Attaches multiple available provider tools to an explicit conversation path.
pub fn attach_tools_with_registry_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
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

    store.insert_attached_tools_at_head(conversation_id, head_message_id, &attached_tools)?;

    Ok(schema_names)
}

/// Inserts one tool schema at the active conversation path.
pub fn insert_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.insert_tool_schema(conversation_id, tool_schema)
}

/// Inserts one tool schema at an explicit conversation path.
pub fn insert_tool_schema_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.insert_tool_schema_at_head(conversation_id, head_message_id, tool_schema)
}

/// Updates one tool schema at the active conversation path.
pub fn update_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    current_name: &ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.update_tool_schema(conversation_id, current_name, tool_schema)
}

/// Updates one tool schema at an explicit conversation path.
pub fn update_tool_schema_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    current_name: &ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.update_tool_schema_at_head(conversation_id, head_message_id, current_name, tool_schema)
}

/// Removes one tool schema at the active conversation path.
pub fn remove_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    name: &ToolSchemaName,
) -> Result<()> {
    store.remove_tool_schema(conversation_id, name)
}

/// Removes one tool schema at an explicit conversation path.
pub fn remove_tool_schema_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    name: &ToolSchemaName,
) -> Result<()> {
    store.remove_tool_schema_at_head(conversation_id, head_message_id, name)
}

/// Detaches one model-facing tool schema from a conversation.
pub fn detach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    schema_name: &ToolSchemaName,
) -> Result<()> {
    remove_tool_schema(store, conversation_id, schema_name)
}

/// Detaches one model-facing tool schema from an explicit conversation path.
pub fn detach_tool_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    schema_name: &ToolSchemaName,
) -> Result<()> {
    remove_tool_schema_at_head(store, conversation_id, head_message_id, schema_name)
}
