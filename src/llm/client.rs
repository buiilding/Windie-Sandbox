//! Bifrost Responses HTTP client.

use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use reqwest::Client;

use crate::conversation::Message;
use crate::tool::ToolSchema;

use super::model::{BaseUrl, ModelName};
use super::responses::{ResponsesInputTokensRequest, ResponsesRequest};
use super::serialization::{
    PromptCacheRequest, ReasoningRequest, image_input_detail_for_model, prompt_cache_fields,
    responses_input, responses_tools,
};
use super::stream::{
    AssistantResponse, AssistantStreamState, InputTokenCount, LlmStreamEvent, append_valid_utf8,
    finish_utf8, input_token_count_from_raw, process_final_stream_line, process_stream_lines,
};

/// HTTP client for Bifrost's OpenAI-compatible Responses endpoint.
pub struct BifrostClient {
    http: Client,
    base_url: BaseUrl,
    model: ModelName,
}
impl BifrostClient {
    /// Creates a reusable HTTP client bound to one base URL and model.
    pub fn new(base_url: BaseUrl, model: ModelName) -> Self {
        Self {
            http: Client::new(),
            base_url,
            model,
        }
    }

    /// Builds the Responses endpoint from the normalized base URL.
    pub fn responses_endpoint(&self) -> String {
        format!("{}/responses", self.base_url)
    }

    /// Builds the Responses input-token endpoint from the normalized base URL.
    pub fn input_tokens_endpoint(&self) -> String {
        format!("{}/responses/input_tokens", self.base_url)
    }

    /// Counts the model-facing input tokens for one Responses request.
    ///
    /// The request uses the same message and tool serializers as streaming
    /// inference. This keeps the count endpoint aligned with the payload Windie
    /// would send to Bifrost for a real model turn.
    pub async fn count_input_tokens(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<InputTokenCount> {
        let request = ResponsesInputTokensRequest {
            model: self.model.as_str(),
            input: responses_input(messages, image_input_detail_for_model(self.model.as_str())),
            tools: responses_tools(tools),
        };

        let response = self
            .http
            .post(self.input_tokens_endpoint())
            .json(&request)
            .send()
            .await
            .context("failed to send responses input token request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "responses input token request failed with {status}: {body}"
            ));
        }

        let raw = response
            .json::<serde_json::Value>()
            .await
            .context("failed to parse responses input token response")?;

        input_token_count_from_raw(raw)
    }

    /// Sends the Responses request, streams assistant text deltas to the
    /// caller, and returns the complete assistant response including tool
    /// calls.
    pub async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        reasoning: Option<&ReasoningRequest>,
        prompt_cache: Option<&PromptCacheRequest>,
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        let prompt_cache_fields = prompt_cache_fields(self.model.as_str(), prompt_cache);
        let request = ResponsesRequest {
            model: self.model.as_str(),
            input: responses_input(messages, image_input_detail_for_model(self.model.as_str())),
            tools: responses_tools(tools),
            reasoning,
            prompt_cache_key: prompt_cache_fields.prompt_cache_key,
            prompt_cache_retention: prompt_cache_fields.prompt_cache_retention,
            cache_control: prompt_cache_fields.cache_control,
            stream: true,
        };

        let response = self
            .http
            .post(self.responses_endpoint())
            .json(&request)
            .send()
            .await
            .context("failed to send responses request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("responses request failed with {status}: {body}"));
        }

        let mut stream = response.bytes_stream();
        let mut byte_buffer = Vec::new();
        let mut buffer = String::new();
        let mut state = AssistantStreamState::default();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read responses stream")?;

            // Network chunks can split inside UTF-8 characters or SSE lines, so
            // bytes are decoded separately from line parsing.
            byte_buffer.extend_from_slice(&chunk);
            append_valid_utf8(&mut byte_buffer, &mut buffer)?;
            process_stream_lines(&mut buffer, &mut state, &mut handle_delta)?;
        }

        finish_utf8(&mut byte_buffer, &mut buffer)?;
        process_final_stream_line(&mut buffer, &mut state, &mut handle_delta)?;

        let response = state.finalize()?;
        if response.content.trim().is_empty() && response.metadata.is_empty() {
            return Err(anyhow!(
                "responses stream did not include assistant content or metadata"
            ));
        }

        Ok(response)
    }
}
