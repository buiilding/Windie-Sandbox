//! Tool catalog and tree-wide tool mutation API handlers.

use super::*;
use crate::tool_provider::ProviderManifest;

#[derive(Debug, Serialize)]
pub(super) struct ToolCatalogResponse {
    pub(super) tools: Vec<ToolDefinition>,
    pub(super) providers: Vec<ToolProviderStatusResponse>,
}

#[derive(Debug, Serialize)]
pub(super) struct ToolProviderStatusResponse {
    pub(super) provider_id: String,
    pub(super) display_name: String,
    pub(super) manifest: ProviderManifest,
    pub(super) available: bool,
    pub(super) tool_count: usize,
    pub(super) error: Option<String>,
}

impl ToolProviderStatusResponse {
    fn from_status(status: ToolProviderStatus) -> Self {
        Self {
            provider_id: status.provider_id.as_str().to_string(),
            display_name: status.display_name,
            manifest: status.manifest,
            available: status.available,
            tool_count: status.tool_count,
            error: status.error,
        }
    }
}

pub(super) async fn list_tools(State(state): State<ApiState>) -> ApiResult<ToolCatalogResponse> {
    let store = open_store(&state)?;
    Ok(Json(ToolCatalogResponse {
        tools: operation::available_tools_with_registry(&store, &state.tool_registry)?,
        providers: operation::enabled_provider_statuses(&store, &state.tool_registry)?
            .into_iter()
            .map(ToolProviderStatusResponse::from_status)
            .collect(),
    }))
}

pub(super) async fn list_provider_tools(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<ToolCatalogResponse> {
    let provider_id = ToolProviderId::new(provider_id);
    let store = open_store(&state)?;

    Ok(Json(ToolCatalogResponse {
        tools: operation::available_provider_tools_with_registry(
            &store,
            &state.tool_registry,
            &provider_id,
        )?,
        providers: operation::enabled_provider_statuses(&store, &state.tool_registry)?
            .into_iter()
            .filter(|status| status.provider_id == provider_id)
            .map(ToolProviderStatusResponse::from_status)
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct ToolApprovalModeRequest {
    pub(super) mode: ToolApprovalMode,
}

#[derive(Debug, Serialize)]
pub(super) struct ToolApprovalModeResponse {
    pub(super) tool_approval_mode: ToolApprovalMode,
}

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
pub(super) struct ToolSchemaRequest {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) parameters: Value,
}

impl ToolSchemaRequest {
    fn into_tool_schema(self) -> ToolSchema {
        ToolSchema {
            name: ToolSchemaName::new(self.name),
            description: self.description,
            parameters: self.parameters,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct ToolSchemaResponse {
    pub(super) name: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ToolSchemasResponse {
    pub(super) names: Vec<String>,
}

#[derive(Debug, Serialize)]
/// Read-only list of one conversation's attached tools (model-facing schemas).
pub(super) struct AttachedToolsResponse {
    pub(super) tools: Vec<ToolSchema>,
}

/// Loads one conversation's attached tools without the full inspection report.
pub(super) async fn list_attached_tools(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<AttachedToolsResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = open_store(&state)?;
    let tools = store.load_tool_schemas(&conversation_id)?;

    Ok(Json(AttachedToolsResponse { tools }))
}

#[derive(Debug, Deserialize)]
pub(super) struct AttachToolRequest {
    pub(super) provider_id: String,
    pub(super) tool_name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AttachToolsRequest {
    pub(super) tools: Vec<AttachToolRequest>,
}

pub(super) async fn attach_tool(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let provider_id = ToolProviderId::new(request.provider_id);
    let tool_name = ProviderToolName::new(request.tool_name);
    let mut store = open_store(&state)?;
    let schema_name = operation::attach_tool_with_registry(
        &mut store,
        &conversation_id,
        &provider_id,
        &tool_name,
        &state.tool_registry,
    )?;

    Ok(Json(ToolSchemaResponse {
        name: schema_name.as_str().to_string(),
    }))
}

pub(super) async fn attach_tools(
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
    let mut store = open_store(&state)?;
    let schema_names = operation::attach_tools_with_registry(
        &mut store,
        &conversation_id,
        &requests,
        &state.tool_registry,
    )?;

    Ok(Json(ToolSchemasResponse {
        names: schema_names
            .into_iter()
            .map(|name| name.as_str().to_string())
            .collect(),
    }))
}

pub(super) async fn insert_tool_schema(
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

pub(super) async fn update_tool_schema(
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

pub(super) async fn remove_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let name = ToolSchemaName::new(name);
    let mut store = open_store(&state)?;

    operation::remove_tool_schema(&mut store, &conversation_id, &name)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

pub(super) async fn detach_tool(
    State(state): State<ApiState>,
    Path((conversation_id, schema_name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let schema_name = ToolSchemaName::new(schema_name);
    let mut store = open_store(&state)?;

    operation::detach_tool(&mut store, &conversation_id, &schema_name)?;

    Ok(Json(DeletedResponse { deleted: true }))
}
