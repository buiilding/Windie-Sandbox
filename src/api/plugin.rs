//! Marketplace plugin API handlers.
//!
//! These handlers expose marketplace discovery and plugin lifecycle operations.
//! Production requests use the configured upstream index; tests may inject the
//! checked-in index as a deterministic fixture.

use super::*;

#[derive(Debug, Serialize)]
pub(super) struct PluginMarketplaceResponse {
    pub(super) index: crate::plugin::MarketplaceIndex,
    /// The base URL lets clients resolve relative presentation asset URLs.
    pub(super) source_url: Option<String>,
    pub(super) installed: Vec<InstalledPluginSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct InstalledPluginSummary {
    pub(super) id: String,
    pub(super) version: String,
    pub(super) components: Vec<InstalledPluginComponentSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct InstalledPluginComponentSummary {
    pub(super) id: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
}

#[derive(Debug, Serialize)]
pub(super) struct PluginInstallResponse {
    pub(super) id: String,
    pub(super) version: String,
    pub(super) providers: Vec<operation::ProviderInstallation>,
}

pub(super) async fn list_plugins(
    State(state): State<ApiState>,
) -> ApiResult<PluginMarketplaceResponse> {
    let index = load_marketplace_index(&state).await?;
    let installed = state
        .plugin_store
        .installed_plugins()?
        .into_iter()
        .map(|plugin| InstalledPluginSummary {
            id: plugin.manifest.plugin.id,
            version: plugin.manifest.plugin.version,
            components: plugin
                .manifest
                .components
                .into_iter()
                .map(|component| InstalledPluginComponentSummary {
                    id: component.id,
                    kind: component.kind.to_string(),
                })
                .collect(),
        })
        .collect();

    Ok(Json(PluginMarketplaceResponse {
        index,
        source_url: state.marketplace_index_url.clone(),
        installed,
    }))
}

pub(super) async fn get_plugin(
    State(state): State<ApiState>,
    Path(plugin_id): Path<String>,
) -> ApiResult<InstalledPluginSummary> {
    let plugin = state
        .plugin_store
        .installed_plugins()?
        .into_iter()
        .find(|plugin| plugin.manifest.plugin.id == plugin_id)
        .ok_or_else(|| windie_error::not_found(format!("plugin does not exist: {plugin_id}")))?;

    Ok(Json(InstalledPluginSummary {
        id: plugin.manifest.plugin.id,
        version: plugin.manifest.plugin.version,
        components: plugin
            .manifest
            .components
            .into_iter()
            .map(|component| InstalledPluginComponentSummary {
                id: component.id,
                kind: component.kind.to_string(),
            })
            .collect(),
    }))
}

pub(super) async fn install_plugin(
    State(state): State<ApiState>,
    Path(plugin_id): Path<String>,
) -> ApiResult<PluginInstallResponse> {
    let marketplace = load_marketplace_index(&state).await?;
    marketplace
        .plugins
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| {
            windie_error::not_found(format!(
                "plugin is not listed in the marketplace: {plugin_id}"
            ))
        })?;

    let plugin_store = state.plugin_store.clone();
    let registry = state.tool_registry.clone();
    let store_path = state.store_path.clone();
    let marketplace_index_url = state.marketplace_index_url.clone();

    Ok(Json(
        tokio::task::spawn_blocking(move || {
            let plugin = match marketplace_index_url.as_deref() {
                Some(index_url) => crate::plugin::MarketplaceInstaller::default()
                    .install_from_index(&plugin_store, index_url, &marketplace, &plugin_id)?,
                None => plugin_store.install_bundled(&plugin_id)?,
            };
            registry.register_plugin(&plugin)?;
            let store = match store_path.as_ref() {
                Some(path) => Store::open_at(path),
                None => Store::open(),
            }?;

            let mut providers = Vec::new();
            for component in crate::mcp::load_components(&plugin)? {
                let provider_id = ToolProviderId::new(component.component_id);
                providers.push(operation::setup_provider(&store, &registry, &provider_id)?);
            }

            Ok::<_, anyhow::Error>(PluginInstallResponse {
                id: plugin.manifest.plugin.id,
                version: plugin.manifest.plugin.version,
                providers,
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("plugin installation worker failed: {error}"))??,
    ))
}

async fn load_marketplace_index(
    state: &ApiState,
) -> anyhow::Result<crate::plugin::MarketplaceIndex> {
    let Some(index_url) = state.marketplace_index_url.clone() else {
        return Ok(crate::plugin::bundled_index()?);
    };
    Ok(tokio::task::spawn_blocking(move || {
        crate::plugin::MarketplaceInstaller::default().fetch_index(&index_url)
    })
    .await
    .map_err(|error| anyhow::anyhow!("marketplace index worker failed: {error}"))??)
}

pub(super) async fn uninstall_plugin(
    State(state): State<ApiState>,
    Path(plugin_id): Path<String>,
) -> ApiResult<DeletedResponse> {
    let installed = state
        .plugin_store
        .installed_plugins()?
        .into_iter()
        .filter(|plugin| plugin.manifest.plugin.id == plugin_id)
        .collect::<Vec<_>>();
    installed
        .first()
        .ok_or_else(|| windie_error::not_found(format!("plugin does not exist: {plugin_id}")))?;

    for plugin in &installed {
        for component in crate::mcp::load_components(plugin)? {
            let provider_id = ToolProviderId::new(component.component_id);
            state
                .tool_registry
                .stop_provider_sessions(&provider_id)
                .await;
        }
    }

    let plugin_store = state.plugin_store.clone();
    let registry = state.tool_registry.clone();
    let store_path = state.store_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut store = match store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        for plugin in &installed {
            for component in crate::mcp::load_components(plugin)? {
                let provider_id = ToolProviderId::new(component.component_id);
                if store.load_installed_provider(&provider_id)?.is_some() {
                    operation::uninstall_provider(&mut store, &registry, &provider_id)?;
                }
            }
            registry.unregister_plugin(plugin)?;
        }
        plugin_store.remove_plugin(&plugin_id)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|error| anyhow::anyhow!("plugin uninstall worker failed: {error}"))??;

    Ok(Json(DeletedResponse { deleted: true }))
}
