//! Provider-manager lifecycle API handlers.
//!
//! These handlers expose persisted provider state and explicit health checks.
//! Blocking provider provisioning is moved off the async request worker so a
//! download, PowerShell installer, or MCP catalog handshake cannot stall the
//! rest of the localhost API.

use super::*;

#[derive(Debug, Default, Deserialize)]
pub(super) struct ProviderSetupRequest {
    pub(super) chrome_devtools_mode: Option<crate::mcp::ChromeDevToolsConnectionMode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProviderConfigurationRequest {
    pub(super) chrome_devtools_mode: crate::mcp::ChromeDevToolsConnectionMode,
}

pub(super) async fn list_providers(
    State(state): State<ApiState>,
) -> ApiResult<Vec<operation::ProviderInstallation>> {
    let store = open_store(&state)?;

    Ok(Json(operation::list_provider_installations(
        &store,
        &state.tool_registry,
    )?))
}

pub(super) async fn chrome_devtools_remote_debugging(
    State(_state): State<ApiState>,
) -> ApiResult<operation::ChromeDevToolsRemoteDebuggingStatus> {
    Ok(Json(
        tokio::task::spawn_blocking(operation::chrome_devtools_remote_debugging_status)
            .await
            .map_err(|error| anyhow::anyhow!("remote debugging check failed: {error}"))?,
    ))
}

pub(super) async fn open_chrome_devtools_remote_debugging() -> ApiResult<serde_json::Value> {
    tokio::task::spawn_blocking(operation::open_chrome_devtools_remote_debugging)
        .await
        .map_err(|error| anyhow::anyhow!("Chrome settings opener failed: {error}"))??;

    Ok(Json(serde_json::json!({ "opened": true })))
}

pub(super) async fn get_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);
    let provider = operation::list_provider_installations(&store, &state.tool_registry)?
        .into_iter()
        .find(|provider| provider.manifest.provider_id == provider_id)
        .ok_or_else(|| {
            windie_error::not_found(format!("provider does not exist: {provider_id}"))
        })?;

    Ok(Json(provider))
}

pub(super) async fn install_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(operation::install_provider(
        &store,
        &state.tool_registry,
        &provider_id,
    )?))
}

pub(super) async fn setup_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
    body: Option<Json<ProviderSetupRequest>>,
) -> ApiResult<operation::ProviderInstallation> {
    let mode = body.and_then(|Json(body)| body.chrome_devtools_mode);
    Ok(Json(
        run_blocking_provider_setup(state, provider_id, mode).await?,
    ))
}

pub(super) async fn configure_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderConfigurationRequest>,
) -> ApiResult<operation::ProviderInstallation> {
    let provider_id_typed = ToolProviderId::new(provider_id.clone());
    state
        .tool_registry
        .stop_provider_sessions(&provider_id_typed)
        .await;
    let store_path = state.store_path;
    let registry = state.tool_registry;
    let mode = body.chrome_devtools_mode;
    Ok(Json(
        tokio::task::spawn_blocking(move || {
            let store = match store_path.as_ref() {
                Some(path) => Store::open_at(path),
                None => Store::open(),
            }?;
            let provider_id = ToolProviderId::new(provider_id);
            operation::configure_provider(&store, &registry, &provider_id, mode)
        })
        .await
        .map_err(|error| anyhow::anyhow!("provider configuration worker failed: {error}"))??,
    ))
}

pub(super) async fn enable_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(operation::enable_provider(
        &store,
        &state.tool_registry,
        &provider_id,
    )?))
}

pub(super) async fn disable_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(operation::disable_provider(
        &store,
        &state.tool_registry,
        &provider_id,
    )?))
}

pub(super) async fn repair_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    Ok(Json(
        run_blocking_provider_operation(state, provider_id, operation::repair_provider).await?,
    ))
}

/// Runs a potentially downloading provider operation on a blocking worker.
async fn run_blocking_provider_operation(
    state: ApiState,
    provider_id: String,
    operation: fn(
        &Store,
        &McpRegistry,
        &ToolProviderId,
    ) -> anyhow::Result<operation::ProviderInstallation>,
) -> anyhow::Result<operation::ProviderInstallation> {
    let store_path = state.store_path;
    let registry = state.tool_registry;
    tokio::task::spawn_blocking(move || {
        let store = match store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        let provider_id = ToolProviderId::new(provider_id);
        operation(&store, &registry, &provider_id)
    })
    .await
    .map_err(|error| anyhow::anyhow!("provider operation worker failed: {error}"))?
}

/// Runs provider setup with its optional connection mode on a blocking worker.
async fn run_blocking_provider_setup(
    state: ApiState,
    provider_id: String,
    mode: Option<crate::mcp::ChromeDevToolsConnectionMode>,
) -> anyhow::Result<operation::ProviderInstallation> {
    let store_path = state.store_path;
    let registry = state.tool_registry;
    tokio::task::spawn_blocking(move || {
        let store = match store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        let provider_id = ToolProviderId::new(provider_id);
        operation::setup_provider_with_mode(&store, &registry, &provider_id, mode)
    })
    .await
    .map_err(|error| anyhow::anyhow!("provider setup worker failed: {error}"))?
}

pub(super) async fn health_check_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    Ok(Json(
        run_blocking_provider_operation(state, provider_id, operation::health_check_provider)
            .await?,
    ))
}

pub(super) async fn uninstall_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<DeletedResponse> {
    let provider_id = ToolProviderId::new(provider_id);
    state
        .tool_registry
        .stop_provider_sessions(&provider_id)
        .await;

    let store_path = state.store_path;
    let registry = state.tool_registry;
    tokio::task::spawn_blocking(move || {
        let mut store = match store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        operation::uninstall_provider(&mut store, &registry, &provider_id)
    })
    .await
    .map_err(|error| anyhow::anyhow!("provider uninstall worker failed: {error}"))??;

    Ok(Json(DeletedResponse { deleted: true }))
}
