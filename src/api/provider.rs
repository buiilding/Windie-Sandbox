//! Provider-manager lifecycle API handlers.
//!
//! These handlers expose persisted provider state and explicit health checks.
//! Blocking provider provisioning is moved off the async request worker so a
//! download, PowerShell installer, or MCP catalog handshake cannot stall the
//! rest of the localhost API.

use super::*;

pub(super) async fn list_providers(
    State(state): State<ApiState>,
) -> ApiResult<Vec<operation::ProviderInstallation>> {
    let store = open_store(&state)?;

    Ok(Json(operation::list_provider_installations(
        &store,
        &state.tool_registry,
    )?))
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
) -> ApiResult<operation::ProviderInstallation> {
    Ok(Json(
        run_blocking_provider_operation(state, provider_id, operation::setup_provider).await?,
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
        &ToolProviderRegistry,
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
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    operation::uninstall_provider(&store, &state.tool_registry, &provider_id)?;

    Ok(Json(DeletedResponse { deleted: true }))
}
