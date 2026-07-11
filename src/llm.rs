//! LLM contracts and Bifrost client facade.
//!
//! Model discovery and Responses wire behavior live in focused child modules.
//! Runtime code depends on the typed contracts and `BifrostClient` exposed
//! here, without depending on provider wire types.

mod client;
mod models;
mod responses;
mod stream;

pub use models::{list_models, model_parameters};

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::conversation::{Message, MessageMetadata, ToolSchema};

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

/// Minimal LLM interface needed by runtime query execution.
///
/// Tests use this trait to simulate success and failure without making network
/// requests.
pub(crate) trait RuntimeLlm {
    async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        reasoning: Option<&ReasoningRequest>,
        prompt_cache: Option<&PromptCacheRequest>,
        handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq)]
/// Complete assistant response received from a streaming Responses request.
pub struct AssistantResponse {
    pub content: String,
    pub metadata: MessageMetadata,
    pub finish_reason: Option<FinishReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
/// One live event emitted while parsing a streamed assistant response.
///
/// These events are display-only. `AssistantStreamState` still assembles the
/// final assistant content and metadata, and runtime persistence uses that
/// complete response as the source of truth.
pub enum LlmStreamEvent<'a> {
    /// Natural-language assistant output text.
    AssistantDelta(&'a str),
    /// Reasoning-summary text reported by providers that expose it.
    ReasoningDelta(&'a str),
    /// Incremental function-call metadata or argument text.
    ToolCallDelta {
        index: u16,
        id: Option<&'a str>,
        name: Option<&'a str>,
        arguments_delta: Option<&'a str>,
    },
}

#[derive(Debug, Clone, PartialEq)]
/// Token count returned by Bifrost's Responses input-token endpoint.
///
/// `input_tokens` is the model-facing input count for the request Windie built.
/// Bifrost may also return provider-specific totals or breakdown fields, so the
/// full response is preserved for future UI or persistence work.
pub struct InputTokenCount {
    pub input_tokens: u64,
    pub total_tokens: Option<u64>,
    pub model: Option<String>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
/// Result of asking Bifrost to count Responses input tokens.
///
/// Some routed providers do not implement Bifrost's count endpoint. Windie keeps
/// that as a typed outcome so client layers do not need to inspect provider
/// error payloads.
pub enum InputTokenCountOutcome {
    Count(InputTokenCount),
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Normalized reason the provider stopped the assistant stream.
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Optional normalized reasoning controls sent to Bifrost.
///
/// Windie stores no model-specific reasoning table. Clients choose a value that
/// came from Bifrost model-parameter metadata, and `llm.rs` serializes it into
/// the OpenAI-compatible `reasoning` object Bifrost already understands.
pub struct ReasoningRequest {
    /// Model-specific reasoning effort selected by the user/client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Optional visible reasoning-summary mode for OpenAI Responses models.
    ///
    /// This is separate from `effort`: a model can spend hidden reasoning
    /// tokens without returning displayable summary text unless this field is
    /// requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl ReasoningRequest {
    /// Returns whether this request would serialize any provider-facing data.
    pub fn is_empty(&self) -> bool {
        self.effort.is_none() && self.summary.is_none()
    }
}

/// Converts a client-selected reasoning setting into the provider request shape
/// for one concrete Bifrost model.
///
/// OpenAI Responses models need an explicit visible-summary request before they
/// stream reasoning-summary deltas. Other providers receive only the normalized
/// fields the client selected.
fn provider_name(model: &str) -> Option<&str> {
    model.split_once('/').map(|(provider, _)| provider)
}

/// Returns the provider-local model name used by Bifrost metadata and image
/// capability checks.
fn provider_local_model_name(model: &str) -> &str {
    model
        .rsplit_once('/')
        .map(|(_, local_model)| local_model)
        .unwrap_or(model)
}

pub fn reasoning_request_for_model(
    model: &ModelName,
    reasoning: Option<ReasoningRequest>,
) -> Option<ReasoningRequest> {
    let mut reasoning = reasoning.filter(|reasoning| !reasoning.is_empty())?;

    if provider_name(model.as_str()) == Some("openai")
        && reasoning.effort.is_some()
        && reasoning.summary.is_none()
    {
        reasoning.summary = Some("auto".to_string());
    }

    Some(reasoning)
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Provider prompt-cache hint for one model request.
///
/// Windie owns conversation identity, so it creates the stable cache key. The
/// provider-specific wire mapping stays in this module: OpenAI receives
/// `prompt_cache_key` fields, while Anthropic receives `cache_control`.
pub struct PromptCacheRequest {
    /// Stable provider cache key for the repeated prompt prefix.
    pub key: String,
    /// Optional provider retention hint. OpenAI-compatible providers use this;
    /// Anthropic cache-control markers ignore it.
    pub retention: Option<String>,
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

/// HTTP client for Bifrost's OpenAI-compatible Responses endpoint.
pub struct BifrostClient {
    http: Client,
    base_url: BaseUrl,
    model: ModelName,
}

#[cfg(test)]
mod tests;
