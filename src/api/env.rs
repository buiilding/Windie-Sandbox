//! Provider-secret environment API handlers.
//!
//! These handlers let onboarding clients persist provider secrets into
//! Windie's `~/.windie/.env` file. Writes are scoped to environment keys that
//! installed-provider manifests declare as secrets, so the localhost API
//! cannot edit arbitrary user environment values.

use std::collections::{HashMap, HashSet};

use super::*;

#[derive(Debug, Deserialize)]
/// Request body for writing provider-secret environment values.
pub(super) struct SetEnvValuesRequest {
    pub(super) assignments: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
/// Result of one provider-secret environment write.
pub(super) struct SetEnvValuesResponse {
    pub(super) updated: usize,
}

/// Writes manifest-declared provider secrets into `~/.windie/.env`.
pub(super) async fn set_env_values_handler(
    State(state): State<ApiState>,
    Json(request): Json<SetEnvValuesRequest>,
) -> ApiResult<SetEnvValuesResponse> {
    if request.assignments.is_empty() {
        return Err(windie_error::invalid_request(
            "at least one environment assignment is required",
        )
        .into());
    }

    // Only keys declared as secrets by a known provider manifest may be
    // written through the API. This keeps the localhost boundary aligned with
    // provider onboarding instead of becoming a general `.env` editor.
    let allowed: HashSet<String> = state
        .tool_registry
        .provider_manifests()
        .iter()
        .flat_map(|manifest| manifest.secrets.iter().map(|secret| secret.env_key.clone()))
        .collect();

    let mut assignments = Vec::with_capacity(request.assignments.len());
    for (key, value) in request.assignments {
        if !allowed.contains(&key) {
            return Err(windie_error::invalid_request(format!(
                "environment key is not a declared provider secret: {key}"
            ))
            .into());
        }
        if value.trim().is_empty() {
            return Err(
                windie_error::invalid_request(format!("value for {key} cannot be empty")).into(),
            );
        }
        assignments.push((key, value));
    }

    local::set_env_values(&assignments)?;

    Ok(Json(SetEnvValuesResponse {
        updated: assignments.len(),
    }))
}
