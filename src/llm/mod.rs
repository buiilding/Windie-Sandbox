//! OpenAI-compatible Bifrost client boundary.
//!
//! This module owns provider HTTP request serialization, HTTP requests to
//! Bifrost's Responses and model metadata endpoints, and streamed Responses
//! event parsing. Runtime code passes Windie messages and tool schemas in; this
//! boundary turns them into provider wire shapes and back into Windie response
//! types.

mod client;
mod error;
pub mod gateway;
mod management;
mod model;
mod responses;
mod serialization;
mod stream;

pub use client::BifrostClient;
pub use error::{LlmError, LlmErrorKind};
pub use management::{
    BifrostManagementClient, CreateProviderKey, ProviderCatalog, ProviderCatalogEntry, ProviderKey,
    ProviderKeyList,
};
pub use model::{
    BaseUrl, ModelInfo, ModelName, ModelParameter, ModelParameterOption, list_models,
    model_parameters,
};
pub use serialization::{PromptCacheRequest, ReasoningRequest};
pub use stream::{AssistantResponse, InputTokenCount, LlmStreamEvent};

#[cfg(test)]
pub use model::ModelParameterInfo;
#[cfg(test)]
pub use stream::FinishReason;

use anyhow::{Context, Result};

use crate::conversation::Message;
use crate::tool::ToolSchema;

/// Serializes one provider-facing Responses request without performing HTTP.
///
/// The benchmark boundary uses the same serializer as `BifrostClient::stream`
/// so request construction is measured independently from SQLite and network
/// latency.
pub(crate) fn benchmark_responses_request_size(
    model: &str,
    messages: &[Message],
    tools: &[ToolSchema],
) -> Result<usize> {
    let prompt_cache = None;
    let prompt_cache_fields = serialization::prompt_cache_fields(model, prompt_cache);
    let request = responses::ResponsesRequest {
        model,
        input: serialization::responses_input(
            messages,
            serialization::image_input_detail_for_model(model),
        ),
        tools: serialization::responses_tools(tools),
        reasoning: None,
        prompt_cache_key: prompt_cache_fields.prompt_cache_key,
        prompt_cache_retention: prompt_cache_fields.prompt_cache_retention,
        cache_control: prompt_cache_fields.cache_control,
        stream: true,
    };

    serde_json::to_vec(&request)
        .context("failed to serialize benchmark Responses request")
        .map(|bytes| bytes.len())
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

impl RuntimeLlm for BifrostClient {
    async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        reasoning: Option<&ReasoningRequest>,
        prompt_cache: Option<&PromptCacheRequest>,
        handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        BifrostClient::stream(self, messages, tools, reasoning, prompt_cache, handle_delta).await
    }
}

#[cfg(test)]
mod tests;
