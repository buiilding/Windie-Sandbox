//! OpenAI-compatible Bifrost client boundary.
//!
//! This module owns provider HTTP request serialization, HTTP requests to
//! Bifrost's Responses and model metadata endpoints, and streamed Responses
//! event parsing. Runtime code passes Windie messages and tool schemas in; this
//! boundary turns them into provider wire shapes and back into Windie response
//! types.

mod client;
mod model;
mod responses;
mod serialization;
mod stream;

pub use client::BifrostClient;
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

use anyhow::Result;

use crate::conversation::Message;
use crate::tool::ToolSchema;

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
