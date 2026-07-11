//! HTTP orchestration for Bifrost Responses endpoints.

use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;

use super::responses::{input_tokens_request, responses_request};
use super::stream::ResponseStreamDecoder;
use super::{
    AssistantResponse, BaseUrl, BifrostClient, Client, InputTokenCount, InputTokenCountOutcome,
    LlmStreamEvent, Message, ModelName, PromptCacheRequest, ReasoningRequest, RuntimeLlm,
    ToolSchema,
};

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
    ) -> Result<InputTokenCountOutcome> {
        let request = input_tokens_request(&self.model, messages, tools);

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
            if is_unsupported_input_token_count_response(&body) {
                return Ok(InputTokenCountOutcome::Unsupported);
            }

            return Err(anyhow!(
                "responses input token request failed with {status}: {body}"
            ));
        }

        let raw = response
            .json::<serde_json::Value>()
            .await
            .context("failed to parse responses input token response")?;

        input_token_count_from_raw(raw).map(InputTokenCountOutcome::Count)
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
        let request = responses_request(&self.model, messages, tools, reasoning, prompt_cache);

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
        let mut decoder = ResponseStreamDecoder::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read responses stream")?;

            decoder.push(&chunk, &mut handle_delta)?;
        }

        let response = decoder.finish(&mut handle_delta)?;
        if response.content.trim().is_empty() && response.metadata.is_empty() {
            return Err(anyhow!(
                "responses stream did not include assistant content or metadata"
            ));
        }

        Ok(response)
    }
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

/// Extracts the stable count fields from Bifrost's input-token response.
fn input_token_count_from_raw(raw: serde_json::Value) -> Result<InputTokenCount> {
    let input_tokens = raw
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("responses input token response missing input_tokens"))?;
    let total_tokens = raw.get("total_tokens").and_then(serde_json::Value::as_u64);
    let model = raw
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    Ok(InputTokenCount {
        input_tokens,
        total_tokens,
        model,
        raw,
    })
}

/// Returns whether Bifrost reported that the routed provider cannot preflight
/// Responses token counts.
fn is_unsupported_input_token_count_response(body: &str) -> bool {
    let Ok(raw) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let unsupported_operation = raw
        .pointer("/error/code")
        .and_then(serde_json::Value::as_str)
        == Some("unsupported_operation");
    let count_tokens_request = raw
        .pointer("/extra_fields/request_type")
        .and_then(serde_json::Value::as_str)
        == Some("count_tokens");
    let unsupported_message = raw
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|message| message.contains("count_tokens is not supported"));

    unsupported_message || (unsupported_operation && count_tokens_request)
}

#[cfg(test)]
mod tests;
