//! Tool registry, attachment, schema, policy, and direct execution routes.

use super::*;

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/tools", get(list_tools))
        .route("/api/tools/{provider_id}", get(list_provider_tools))
        .route(
            "/api/conversations/{conversation_id}/tool-approval-mode",
            patch(set_tool_approval_mode),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-schemas",
            post(insert_tool_schema),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-schemas/{name}",
            patch(update_tool_schema).delete(remove_tool_schema),
        )
        .route(
            "/api/conversations/{conversation_id}/tools",
            post(attach_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/tools/batch",
            post(attach_tools),
        )
        .route(
            "/api/conversations/{conversation_id}/tools/{schema_name}",
            axum::routing::delete(detach_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals",
            get(list_approvals),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/approve",
            post(approve_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/deny",
            post(deny_tool),
        )
}

#[derive(Debug, Serialize)]
/// API response for provider tools available to attach.
struct ToolCatalogResponse {
    tools: Vec<ToolDefinition>,
}

/// Lists provider tools clients may attach to conversations.
async fn list_tools(State(state): State<ApiState>) -> ApiResult<ToolCatalogResponse> {
    let registry = Arc::clone(&state.tool_registry);
    let tools =
        tokio::task::spawn_blocking(move || operation::available_tools_with_registry(&registry))
            .await
            .context("tool catalog task stopped")??;
    Ok(Json(ToolCatalogResponse { tools }))
}

/// Lists available tools for one provider.
async fn list_provider_tools(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<ToolCatalogResponse> {
    let provider_id = ToolProviderId::new(provider_id);
    let registry = Arc::clone(&state.tool_registry);
    let tools = tokio::task::spawn_blocking(move || {
        operation::available_provider_tools_with_registry(&registry, &provider_id)
    })
    .await
    .context("provider tool catalog task stopped")??;

    Ok(Json(ToolCatalogResponse { tools }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation-level tool approval mode.
struct ToolApprovalModeRequest {
    mode: ToolApprovalMode,
}

#[derive(Debug, Serialize)]
/// Response for tool approval mode mutation.
struct ToolApprovalModeResponse {
    tool_approval_mode: ToolApprovalMode,
}

/// Sets the conversation default for attached tool approvals.
async fn set_tool_approval_mode(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolApprovalModeRequest>,
) -> ApiResult<ToolApprovalModeResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::set_tool_approval_mode(&mut store, &conversation_id, request.mode)?;

    Ok(Json(ToolApprovalModeResponse {
        tool_approval_mode: store.tool_approval_mode(&conversation_id)?,
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for creating or updating a tool schema.
struct ToolSchemaRequest {
    name: String,
    description: String,
    parameters: Value,
}

impl ToolSchemaRequest {
    /// Converts API JSON into the typed tool schema contract.
    fn into_tool_schema(self) -> ToolSchema {
        ToolSchema {
            name: ToolSchemaName::new(self.name),
            description: self.description,
            parameters: self.parameters,
        }
    }
}

#[derive(Debug, Serialize)]
/// Response for tool schema mutations.
struct ToolSchemaResponse {
    name: String,
}

#[derive(Debug, Serialize)]
/// Response for batch tool schema mutations.
struct ToolSchemasResponse {
    names: Vec<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for attaching an available provider tool to a conversation.
struct AttachToolRequest {
    provider_id: String,
    tool_name: String,
}

#[derive(Debug, Deserialize)]
/// Request body for attaching multiple available provider tools.
struct AttachToolsRequest {
    tools: Vec<AttachToolRequest>,
}

/// Attaches one available provider tool to a conversation.
async fn attach_tool(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let provider_id = ToolProviderId::new(request.provider_id);
    let tool_name = ProviderToolName::new(request.tool_name);
    let schema_name = tokio::task::spawn_blocking(move || {
        let mut store = open_store(&state)?;
        operation::attach_tool_with_registry(
            &mut store,
            &conversation_id,
            &provider_id,
            &tool_name,
            &state.tool_registry,
        )
    })
    .await
    .context("tool attachment task stopped")??;

    Ok(Json(ToolSchemaResponse {
        name: schema_name.as_str().to_string(),
    }))
}

/// Attaches multiple available provider tools to a conversation.
async fn attach_tools(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolsRequest>,
) -> ApiResult<ToolSchemasResponse> {
    let conversation_id = ConversationId::new(conversation_id);
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
    let schema_names = tokio::task::spawn_blocking(move || {
        let mut store = open_store(&state)?;
        operation::attach_tools_with_registry(
            &mut store,
            &conversation_id,
            &requests,
            &state.tool_registry,
        )
    })
    .await
    .context("tool attachment task stopped")??;

    Ok(Json(ToolSchemasResponse {
        names: schema_names
            .into_iter()
            .map(|name| name.as_str().to_string())
            .collect(),
    }))
}

/// Inserts one conversation-level tool schema.
async fn insert_tool_schema(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_schema = request.into_tool_schema();
    let mut store = open_store(&state)?;

    operation::insert_tool_schema(&mut store, &conversation_id, &tool_schema)?;

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Updates one conversation-level tool schema.
async fn update_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let current_name = ToolSchemaName::new(name);
    let tool_schema = request.into_tool_schema();
    let mut store = open_store(&state)?;

    operation::update_tool_schema(&mut store, &conversation_id, &current_name, &tool_schema)?;

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Removes one conversation-level tool schema.
async fn remove_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let name = ToolSchemaName::new(name);
    let mut store = open_store(&state)?;

    operation::remove_tool_schema(&mut store, &conversation_id, &name)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Detaches one provider-backed tool schema from a conversation.
async fn detach_tool(
    State(state): State<ApiState>,
    Path((conversation_id, schema_name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let schema_name = ToolSchemaName::new(schema_name);
    let mut store = open_store(&state)?;

    operation::detach_tool(&mut store, &conversation_id, &schema_name)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

#[derive(Debug, Serialize)]
/// Response body for pending tool approvals.
struct ApprovalListResponse {
    approvals: Vec<ApprovalResponse>,
}

#[derive(Debug, Serialize)]
/// One pending approval returned to UI clients.
struct ApprovalResponse {
    assistant_message_id: String,
    tool_call_id: String,
    tool_name: String,
    arguments: String,
    reason: String,
}

impl From<ToolApprovalRequest> for ApprovalResponse {
    fn from(approval: ToolApprovalRequest) -> Self {
        Self {
            assistant_message_id: approval.assistant_message_id.as_str().to_string(),
            tool_call_id: approval.tool_call.id.as_str().to_string(),
            tool_name: approval.tool_call.name().to_string(),
            arguments: approval.tool_call.arguments().to_string(),
            reason: approval.reason,
        }
    }
}

/// Lists pending tool calls waiting for approval.
async fn list_approvals(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<ApprovalListResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = open_store(&state)?;
    let approvals = operation::list_tool_approvals_with_registry(
        &store,
        &conversation_id,
        &state.tool_registry,
    )?
    .into_iter()
    .map(ApprovalResponse::from)
    .collect();

    Ok(Json(ApprovalListResponse { approvals }))
}

#[derive(Debug, Serialize)]
/// Response for resolving one pending tool call without continuing the model run.
struct ToolExecutionResponse {
    tool_call_id: String,
    tool_name: String,
    content: String,
    success: bool,
}

impl From<ToolExecutionResult> for ToolExecutionResponse {
    fn from(result: ToolExecutionResult) -> Self {
        Self {
            tool_call_id: result.tool_call_id.as_str().to_string(),
            tool_name: result.tool_name,
            content: result.content,
            success: result.success,
        }
    }
}

/// Executes one approved pending tool call and persists its result.
async fn approve_tool(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolExecutionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = open_store(&state)?;
    let result = operation::approve_tool_with_registry(
        &mut store,
        &conversation_id,
        &tool_call_id,
        &state.tool_registry,
    )
    .await?;

    Ok(Json(result.into()))
}

/// Stores a rejected result for one pending tool call.
async fn deny_tool(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolExecutionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = open_store(&state)?;
    let result = operation::deny_tool(&mut store, &conversation_id, &tool_call_id)?;

    Ok(Json(result.into()))
}
