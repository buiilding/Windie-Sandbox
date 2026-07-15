//! Bifrost model identity, model listing, and model-parameter metadata.

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Base URL for the OpenAI-compatible provider adapter.
pub struct BaseUrl(String);

impl BaseUrl {
    /// Stores the URL without a trailing slash so endpoint joining is stable.
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into().trim_end_matches('/').to_string())
    }

    /// Returns the normalized URL text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BaseUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Provider-qualified model name passed through to Bifrost.
pub struct ModelName(String);

impl ModelName {
    /// Wraps model text as a type so model arguments are not confused with
    /// general strings.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the exact model name sent to Bifrost.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// One model returned by Bifrost's OpenAI-compatible `/models` endpoint.
pub struct ModelInfo {
    pub id: String,
    pub context_length: Option<u64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
/// OpenAI-compatible model list response returned by Bifrost.
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
/// Raw Bifrost model-parameter response for one selected model.
///
/// Bifrost owns the provider/model metadata. Windie keeps the raw response for
/// inspection and extracts only the small effort selector needed by the local
/// developer UI.
pub struct ModelParameterInfo {
    #[serde(default)]
    pub model_parameters: Vec<ModelParameter>,
    pub supports_reasoning: Option<bool>,
    pub supports_reasoning_with_tool_calls: Option<bool>,
    pub supports_prompt_caching: Option<bool>,
    #[serde(skip)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
/// One Bifrost model parameter description.
pub struct ModelParameter {
    pub id: String,
    pub label: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(rename = "accesorKey", alias = "accessorKey")]
    pub accessor_key: Option<String>,
    #[serde(default)]
    pub options: Vec<ModelParameterOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
/// One selectable model-parameter option returned by Bifrost.
pub struct ModelParameterOption {
    pub label: String,
    pub value: String,
}
/// Builds the model-list endpoint from the normalized base URL.
pub(super) fn models_endpoint(base_url: &BaseUrl) -> String {
    format!("{base_url}/models")
}

/// Lists models known to the running Bifrost gateway.
///
/// Provider detection and model discovery are owned by Bifrost. Windie only
/// reads the current gateway state exposed through the OpenAI-compatible
/// `/models` endpoint.
pub async fn list_models(base_url: BaseUrl) -> Result<Vec<ModelInfo>> {
    let response = Client::new()
        .get(models_endpoint(&base_url))
        .send()
        .await
        .context("failed to send model list request")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("model list request failed with {status}: {body}"));
    }

    let response = response
        .json::<ModelsResponse>()
        .await
        .context("failed to parse model list response")?;

    Ok(response.data)
}

/// Loads Bifrost's model-parameter metadata for one model.
///
/// Bifrost's management datasheet can store model rows under different names:
/// a full Windie model ID, a provider-local ID, or sometimes only the final
/// model segment. Windie tries every slash suffix from most-specific to
/// least-specific and uses the first successful metadata row.
pub async fn model_parameters(base_url: BaseUrl, model: &ModelName) -> Result<ModelParameterInfo> {
    let http = Client::new();
    let mut last_error = None;

    for candidate in model_parameter_candidates(model.as_str()) {
        let endpoint = model_parameters_endpoint(&base_url, candidate)?;
        let response = http
            .get(endpoint)
            .send()
            .await
            .context("failed to send model parameter request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            last_error = Some(anyhow!(
                "model parameter request failed with {status}: {body}"
            ));
            continue;
        }

        let raw = response
            .json::<serde_json::Value>()
            .await
            .context("failed to parse model parameter response")?;
        let mut parameters = serde_json::from_value::<ModelParameterInfo>(raw.clone())
            .context("failed to decode model parameter response")?;
        parameters.raw = raw;

        return Ok(parameters);
    }

    Err(last_error.unwrap_or_else(|| anyhow!("model parameter request had no candidates")))
}

/// Builds the Bifrost management endpoint for model parameters.
pub(super) fn model_parameters_endpoint(base_url: &BaseUrl, model: &str) -> Result<Url> {
    let api_root = base_url
        .as_str()
        .strip_suffix("/v1")
        .unwrap_or_else(|| base_url.as_str());
    let mut url = Url::parse(&format!("{api_root}/api/models/parameters"))
        .context("failed to build model parameter endpoint")?;
    url.query_pairs_mut().append_pair("model", model);

    Ok(url)
}

/// Returns model-parameter lookup candidates from most-specific to
/// least-specific.
pub(super) fn model_parameter_candidates(model: &str) -> Vec<&str> {
    let mut candidates = vec![model];
    candidates.extend(
        model
            .char_indices()
            .filter(|(_, character)| *character == '/')
            .filter_map(|(index, _)| {
                let suffix = &model[index + 1..];
                (!suffix.is_empty()).then_some(suffix)
            }),
    );
    candidates.dedup();
    candidates
}
