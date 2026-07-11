//! Bifrost model discovery and model-parameter metadata.

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, Url};
use serde::Deserialize;

use super::{BaseUrl, ModelInfo, ModelName, ModelParameterInfo};

#[derive(Debug, Deserialize)]
/// OpenAI-compatible model list response returned by Bifrost.
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

/// Builds the model-list endpoint from the normalized base URL.
fn models_endpoint(base_url: &BaseUrl) -> String {
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
/// The endpoint is Bifrost-specific management metadata, not an OpenAI
/// compatibility route. Its datasheet mixes routed, provider-local, and bare
/// model identifiers, so Windie tries each representation in that order.
pub async fn model_parameters(
    base_url: BaseUrl,
    model: &ModelName,
) -> Result<Option<ModelParameterInfo>> {
    let http = Client::new();
    for lookup_name in model_parameter_lookup_names(model.as_str()) {
        let endpoint = model_parameters_endpoint(&base_url, lookup_name)?;
        let response = http
            .get(endpoint)
            .send()
            .await
            .context("failed to send model parameter request")?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            continue;
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "model parameter request failed with {status}: {body}"
            ));
        }

        let raw = response
            .json::<serde_json::Value>()
            .await
            .context("failed to parse model parameter response")?;
        let mut parameters = serde_json::from_value::<ModelParameterInfo>(raw.clone())
            .context("failed to decode model parameter response")?;
        parameters.raw = raw;

        return Ok(Some(parameters));
    }

    Ok(None)
}

/// Builds the Bifrost management endpoint for model parameters.
fn model_parameters_endpoint(base_url: &BaseUrl, lookup_name: &str) -> Result<Url> {
    let api_root = base_url
        .as_str()
        .strip_suffix("/v1")
        .unwrap_or_else(|| base_url.as_str());
    let mut url = Url::parse(&format!("{api_root}/api/models/parameters"))
        .context("failed to build model parameter endpoint")?;
    url.query_pairs_mut().append_pair("model", lookup_name);

    Ok(url)
}

/// Returns distinct parameter-datasheet identities from most to least specific.
fn model_parameter_lookup_names(model: &str) -> Vec<&str> {
    let mut names = vec![model];
    if let Some((_, provider_local)) = model.split_once('/')
        && !names.contains(&provider_local)
    {
        names.push(provider_local);
    }
    if let Some((_, bare_model)) = model.rsplit_once('/')
        && !names.contains(&bare_model)
    {
        names.push(bare_model);
    }
    names
}

#[cfg(test)]
mod tests;
