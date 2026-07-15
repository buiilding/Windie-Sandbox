//! Shared API server state and store access helpers.

use super::*;

#[derive(Clone)]
/// Runtime settings captured by the API server at startup.
pub(super) struct ApiState {
    pub(super) gateway_url: String,
    pub(super) base_url: String,
    pub(super) model: String,
    pub(super) api_token: String,
    pub(super) store_path: Option<PathBuf>,
    pub(super) tool_registry: Arc<ToolProviderRegistry>,
    pub(super) session_manager: Arc<SessionManager>,
}

/// Opens the production store, or a test-scoped store when route tests inject
/// one through `ApiState`.
pub(super) fn open_store(state: &ApiState) -> Result<Store> {
    match state.store_path.as_ref() {
        Some(path) => Store::open_at(path),
        None => Store::open(),
    }
}
