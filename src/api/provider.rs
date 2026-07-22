//! Provider-manager lifecycle API handlers.
//!
//! These handlers expose persisted provider state and explicit health checks.
//! They do not install packages yet; package setup belongs to the next phase.

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
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(operation::setup_provider(
        &store,
        &state.tool_registry,
        &provider_id,
    )?))
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
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(operation::repair_provider(
        &store,
        &state.tool_registry,
        &provider_id,
    )?))
}

pub(super) async fn health_check_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<operation::ProviderInstallation> {
    let store = open_store(&state)?;
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(operation::health_check_provider(
        &store,
        &state.tool_registry,
        &provider_id,
    )?))
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
