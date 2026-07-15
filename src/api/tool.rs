//! Tool catalog and branch-scoped tool mutation API handlers.

use super::*;

#[derive(Debug, Serialize)]
/// API response for provider tools available to attach.
pub(super) struct ToolCatalogResponse {
    pub(super) tools: Vec<ToolDefinition>,
    pub(super) providers: Vec<ToolProviderStatusResponse>,
}

#[derive(Debug, Serialize)]
/// Availability status for one approved tool provider.
pub(super) struct ToolProviderStatusResponse {
    pub(super) provider_id: String,
    pub(super) display_name: String,
    pub(super) available: bool,
    pub(super) tool_count: usize,
    pub(super) error: Option<String>,
}

impl ToolProviderStatusResponse {
    fn from_status(status: ToolProviderStatus) -> Self {
        Self {
            provider_id: status.provider_id.as_str().to_string(),
            display_name: status.display_name,
            available: status.available,
            tool_count: status.tool_count,
            error: status.error,
        }
    }
}

/// Lists provider tools clients may attach to conversations.
pub(super) async fn list_tools(State(state): State<ApiState>) -> ApiResult<ToolCatalogResponse> {
    Ok(Json(ToolCatalogResponse {
        tools: operation::available_tools_with_registry(&state.tool_registry)?,
        providers: state
            .tool_registry
            .list_provider_statuses()
            .into_iter()
            .map(ToolProviderStatusResponse::from_status)
            .collect(),
    }))
}

/// Lists available tools for one provider.
pub(super) async fn list_provider_tools(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<ToolCatalogResponse> {
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(ToolCatalogResponse {
        tools: operation::available_provider_tools_with_registry(
            &state.tool_registry,
            &provider_id,
        )?,
        providers: state
            .tool_registry
            .list_provider_statuses()
            .into_iter()
            .filter(|status| status.provider_id == provider_id)
            .map(ToolProviderStatusResponse::from_status)
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation default tool approval mode.
pub(super) struct ToolApprovalModeRequest {
    pub(super) mode: ToolApprovalMode,
}

#[derive(Debug, Serialize)]
/// Response for tool approval mode mutation.
pub(super) struct ToolApprovalModeResponse {
    pub(super) tool_approval_mode: ToolApprovalMode,
}

/// Sets the conversation default for attached tool approvals.
pub(super) async fn set_tool_approval_mode(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolApprovalModeRequest>,
) -> ApiResult<ToolApprovalModeResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::set_tool_approval_mode(&mut store, &conversation_id, request.mode)?;
    drop(store);
    state
        .session_manager
        .resume_waiting_for_conversation(&conversation_id)?;
    let store = open_store(&state)?;

    Ok(Json(ToolApprovalModeResponse {
        tool_approval_mode: store.tool_approval_mode(&conversation_id)?,
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for creating or updating a tool schema.
pub(super) struct ToolSchemaRequest {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) parameters: Value,
    pub(super) head_message_id: Option<String>,
}

impl ToolSchemaRequest {
    /// Converts API JSON into the typed tool schema contract.
    fn into_parts(self) -> (ToolSchema, Option<MessageId>) {
        (
            ToolSchema {
                name: ToolSchemaName::new(self.name),
                description: self.description,
                parameters: self.parameters,
            },
            requested_head_message_id(self.head_message_id),
        )
    }
}

#[derive(Debug, Serialize)]
/// Response for tool schema mutations.
pub(super) struct ToolSchemaResponse {
    pub(super) name: String,
}

#[derive(Debug, Serialize)]
/// Response for batch tool schema mutations.
pub(super) struct ToolSchemasResponse {
    pub(super) names: Vec<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for attaching an available provider tool to a conversation.
pub(super) struct AttachToolRequest {
    pub(super) provider_id: String,
    pub(super) tool_name: String,
    pub(super) head_message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for attaching multiple available provider tools.
pub(super) struct AttachToolsRequest {
    pub(super) tools: Vec<AttachToolRequest>,
    pub(super) head_message_id: Option<String>,
}

/// Attaches one available provider tool to a conversation.
pub(super) async fn attach_tool(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let provider_id = ToolProviderId::new(request.provider_id);
    let tool_name = ProviderToolName::new(request.tool_name);
    let head_message_id = requested_head_message_id(request.head_message_id);
    let mut store = open_store(&state)?;
    let schema_name = match head_message_id.as_ref() {
        Some(head_message_id) => operation::attach_tool_with_registry_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &provider_id,
            &tool_name,
            &state.tool_registry,
        )?,
        None => operation::attach_tool_with_registry(
            &mut store,
            &conversation_id,
            &provider_id,
            &tool_name,
            &state.tool_registry,
        )?,
    };

    Ok(Json(ToolSchemaResponse {
        name: schema_name.as_str().to_string(),
    }))
}

/// Attaches multiple available provider tools to a conversation.
pub(super) async fn attach_tools(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolsRequest>,
) -> ApiResult<ToolSchemasResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = requested_head_message_id(request.head_message_id);
    let requests = request
        .tools
        .into_iter()
        .map(|tool| {
            operation::ToolAttachmentInput::new(
                ToolProviderId::new(tool.provider_id),
                ProviderToolName::new(tool.tool_name),
            )
        })
        .collect::<Vec<_>>();
    let mut store = open_store(&state)?;
    let schema_names = match head_message_id.as_ref() {
        Some(head_message_id) => operation::attach_tools_with_registry_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &requests,
            &state.tool_registry,
        )?,
        None => operation::attach_tools_with_registry(
            &mut store,
            &conversation_id,
            &requests,
            &state.tool_registry,
        )?,
    };

    Ok(Json(ToolSchemasResponse {
        names: schema_names
            .into_iter()
            .map(|name| name.as_str().to_string())
            .collect(),
    }))
}

/// Inserts one tool schema on the active or requested path.
pub(super) async fn insert_tool_schema(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let (tool_schema, head_message_id) = request.into_parts();
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::insert_tool_schema_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &tool_schema,
        )?,
        None => operation::insert_tool_schema(&mut store, &conversation_id, &tool_schema)?,
    }

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Updates one tool schema on the active or requested path.
pub(super) async fn update_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let current_name = ToolSchemaName::new(name);
    let (tool_schema, head_message_id) = request.into_parts();
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::update_tool_schema_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &current_name,
            &tool_schema,
        )?,
        None => operation::update_tool_schema(
            &mut store,
            &conversation_id,
            &current_name,
            &tool_schema,
        )?,
    }

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Removes one tool schema from the active or requested path.
pub(super) async fn remove_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
    Query(query): Query<ContextMutationQuery>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let name = ToolSchemaName::new(name);
    let head_message_id = requested_head_message_id(query.head_message_id);
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::remove_tool_schema_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &name,
        )?,
        None => operation::remove_tool_schema(&mut store, &conversation_id, &name)?,
    }

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Detaches one provider-backed tool schema from a conversation.
pub(super) async fn detach_tool(
    State(state): State<ApiState>,
    Path((conversation_id, schema_name)): Path<(String, String)>,
    Query(query): Query<ContextMutationQuery>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let schema_name = ToolSchemaName::new(schema_name);
    let head_message_id = requested_head_message_id(query.head_message_id);
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::detach_tool_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &schema_name,
        )?,
        None => operation::detach_tool(&mut store, &conversation_id, &schema_name)?,
    }

    Ok(Json(DeletedResponse { deleted: true }))
}
