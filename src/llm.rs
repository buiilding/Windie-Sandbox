//! OpenAI-compatible Responses client.
//!
//! This module owns provider HTTP request serialization, HTTP requests to
//! Bifrost's Responses and model-list endpoints, and streamed Responses event
//! parsing. Runtime code passes Windie messages and tool schemas in; this
//! boundary turns them into the provider wire shape.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

use crate::conversation::{
    ImagePart, Message, MessageMetadata, MessagePart, TokenUsage, ToolCall, ToolSchema,
};

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

#[derive(Debug, Serialize)]
/// JSON request body sent to `/responses`.
struct ResponsesRequest<'a> {
    model: &'a str,
    input: Vec<ResponsesInputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<&'a ReasoningRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_retention: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControlRequest>,
    stream: bool,
}

#[derive(Debug, Serialize)]
/// Anthropic-family prompt-cache control forwarded through Bifrost.
struct CacheControlRequest {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug)]
/// Provider-specific cache fields to include in one Responses request.
struct PromptCacheFields<'a> {
    prompt_cache_key: Option<&'a str>,
    prompt_cache_retention: Option<&'a str>,
    cache_control: Option<CacheControlRequest>,
}

#[derive(Debug, Serialize)]
/// JSON request body sent to `/responses/input_tokens`.
struct ResponsesInputTokensRequest<'a> {
    model: &'a str,
    input: Vec<ResponsesInputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool<'a>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Model-facing image detail level serialized onto every Responses image block.
///
/// Windie stores user images and tool-output images through the same
/// `MessagePart::Image` primitive. Choosing the detail level here keeps visual
/// grounding policy inside the provider HTTP boundary instead of duplicating it
/// across input, MCP, or tool-provider code paths.
enum ImageInputDetail {
    High,
    Original,
}

impl ImageInputDetail {
    /// Returns the OpenAI-compatible wire value for Responses `input_image`.
    fn as_wire_value(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Original => "original",
        }
    }
}

#[derive(Debug, Deserialize)]
/// OpenAI-compatible model list response returned by Bifrost.
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One item inside a Responses `input` array.
enum ResponsesInputItem<'a> {
    Message(ResponsesMessageItem<'a>),
    FunctionCall(ResponsesFunctionCallItem<'a>),
    FunctionCallOutput(ResponsesFunctionCallOutputItem<'a>),
}

#[derive(Debug, Serialize)]
/// User/system/assistant message item for Responses input.
struct ResponsesMessageItem<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    content: ResponsesMessageContent<'a>,
}

#[derive(Debug, Serialize)]
/// Assistant function-call item for Responses input history.
struct ResponsesFunctionCallItem<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    name: &'a str,
    arguments: &'a str,
    status: &'static str,
}

#[derive(Debug, Serialize)]
/// Function-call output item for Responses input history.
struct ResponsesFunctionCallOutputItem<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    output: ResponsesToolOutput<'a>,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// Responses message content: plain text or ordered multimodal blocks.
enum ResponsesMessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ResponsesContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// Responses function-call output: plain text or ordered multimodal blocks.
enum ResponsesToolOutput<'a> {
    Text(&'a str),
    Parts(Vec<ResponsesContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One Responses content block.
enum ResponsesContentPart<'a> {
    Text(ResponsesTextPart<'a>),
    Image(ResponsesImagePart),
}

#[derive(Debug, Serialize)]
/// Responses text content block.
struct ResponsesTextPart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}

#[derive(Debug, Serialize)]
/// Responses image content block.
struct ResponsesImagePart {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: String,
    detail: &'static str,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible function tool definition sent through Responses.
struct ResponsesTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Debug, Deserialize)]
/// One streamed Responses server-sent event payload from Bifrost.
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    output_index: Option<u16>,
    delta: Option<String>,
    text: Option<String>,
    refusal: Option<String>,
    arguments: Option<String>,
    item: Option<ResponsesStreamItem>,
    response: Option<ResponsesStreamResponse>,
    error: Option<ResponsesStreamError>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Stream item used by `response.output_item.*` events.
struct ResponsesStreamItem {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    call_id: Option<String>,
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Terminal response body used by failed/incomplete Responses events.
struct ResponsesStreamResponse {
    error: Option<ResponsesStreamError>,
    usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
/// Error payload embedded in Responses stream events.
struct ResponsesStreamError {
    message: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Default)]
/// In-progress assistant stream state.
struct AssistantStreamState {
    content: String,
    metadata: MessageMetadata,
    tool_calls: BTreeMap<u16, PartialToolCall>,
    finish_reason: Option<FinishReason>,
}

#[derive(Debug, Default)]
/// Tool call assembled from Responses stream events.
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
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
    ) -> Result<InputTokenCountOutcome> {
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

/// Returns the provider-local model name expected by Bifrost management APIs.
fn provider_local_model_name(model: &str) -> &str {
    model
        .rsplit_once('/')
        .map(|(_, local_model)| local_model)
        .unwrap_or(model)
}

/// Builds provider-specific prompt-cache fields for Bifrost's Responses route.
///
/// OpenAI and Anthropic expose different cache controls. Windie keeps one
/// internal cache request and lets this provider HTTP boundary translate it.
/// Unqualified or unsupported provider names intentionally serialize no cache
/// fields because Windie cannot know the correct upstream contract.
fn prompt_cache_fields<'a>(
    model: &str,
    prompt_cache: Option<&'a PromptCacheRequest>,
) -> PromptCacheFields<'a> {
    let Some(prompt_cache) = prompt_cache else {
        return PromptCacheFields {
            prompt_cache_key: None,
            prompt_cache_retention: None,
            cache_control: None,
        };
    };

    match provider_name(model) {
        Some("openai") => PromptCacheFields {
            prompt_cache_key: Some(prompt_cache.key.as_str()),
            prompt_cache_retention: prompt_cache.retention.as_deref(),
            cache_control: None,
        },
        Some("anthropic") => PromptCacheFields {
            prompt_cache_key: None,
            prompt_cache_retention: None,
            cache_control: Some(CacheControlRequest { kind: "ephemeral" }),
        },
        _ => PromptCacheFields {
            prompt_cache_key: None,
            prompt_cache_retention: None,
            cache_control: None,
        },
    }
}

/// Returns the provider prefix from a Bifrost model id such as `openai/gpt-5.5`.
fn provider_name(model: &str) -> Option<&str> {
    model.split_once('/').map(|(provider, _)| provider)
}

/// Chooses the Responses image detail level for one concrete Bifrost model.
///
/// `high` is the provider-unified default because it is broadly understood by
/// OpenAI-compatible vision adapters. `original` is reserved for known OpenAI
/// model names where OpenAI documents pixel-preserving image processing for
/// GUI grounding and computer-use accuracy.
fn image_input_detail_for_model(model: &str) -> ImageInputDetail {
    if provider_name(model) == Some("openai")
        && openai_model_supports_original_image_detail(provider_local_model_name(model))
    {
        return ImageInputDetail::Original;
    }

    ImageInputDetail::High
}

/// Returns whether one OpenAI-local model name supports `detail: original`.
fn openai_model_supports_original_image_detail(model: &str) -> bool {
    let model = model.to_ascii_lowercase();

    if model.starts_with("gpt-5.4-mini") || model.starts_with("gpt-5.4-nano") {
        return false;
    }

    model == "gpt-5.4"
        || model.starts_with("gpt-5.4-")
        || model.starts_with("gpt-5.5")
        || model.starts_with("gpt-5.6")
}

/// Converts Windie's internal messages into the Responses request input array.
fn responses_input(
    messages: &[Message],
    image_detail: ImageInputDetail,
) -> Vec<ResponsesInputItem<'_>> {
    messages
        .iter()
        .flat_map(|message| responses_items_for_message(message, image_detail))
        .collect()
}

/// Converts one Windie message into one or more Responses input items.
fn responses_items_for_message(
    message: &Message,
    image_detail: ImageInputDetail,
) -> Vec<ResponsesInputItem<'_>> {
    match message.role {
        crate::conversation::Role::Assistant => {
            let metadata = message.metadata.as_ref();
            if let Some(tool_calls) = metadata
                .map(|metadata| metadata.tool_calls.as_slice())
                .filter(|tool_calls| !tool_calls.is_empty())
            {
                let mut items = Vec::new();
                if !message.content.is_empty() || !message.parts.is_empty() {
                    items.push(ResponsesInputItem::Message(ResponsesMessageItem {
                        kind: "message",
                        role: "assistant",
                        content: responses_message_content(message, image_detail),
                    }));
                }
                items.extend(
                    tool_calls
                        .iter()
                        .map(|tool_call| {
                            ResponsesInputItem::FunctionCall(ResponsesFunctionCallItem {
                                kind: "function_call",
                                call_id: tool_call.id.as_str(),
                                name: tool_call.name(),
                                arguments: tool_call.arguments(),
                                status: "completed",
                            })
                        })
                        .collect::<Vec<_>>(),
                );

                return items;
            }

            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "assistant",
                content: responses_message_content(message, image_detail),
            })]
        }
        crate::conversation::Role::Tool => {
            let call_id = message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.tool_call_id.as_ref())
                .map(|id| id.as_str());
            call_id
                .map(|call_id| {
                    vec![ResponsesInputItem::FunctionCallOutput(
                        ResponsesFunctionCallOutputItem {
                            kind: "function_call_output",
                            call_id,
                            output: responses_tool_output(message, image_detail),
                            status: "completed",
                        },
                    )]
                })
                .unwrap_or_default()
        }
        crate::conversation::Role::System => {
            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "system",
                content: responses_message_content(message, image_detail),
            })]
        }
        crate::conversation::Role::User => {
            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "user",
                content: responses_message_content(message, image_detail),
            })]
        }
    }
}

/// Converts one normal message body into Responses content.
fn responses_message_content(
    message: &Message,
    image_detail: ImageInputDetail,
) -> ResponsesMessageContent<'_> {
    if message.parts.is_empty() {
        if message.role == crate::conversation::Role::Assistant && !message.content.is_empty() {
            return ResponsesMessageContent::Parts(vec![ResponsesContentPart::Text(
                ResponsesTextPart {
                    kind: "output_text",
                    text: &message.content,
                },
            )]);
        }

        return ResponsesMessageContent::Text(&message.content);
    }

    ResponsesMessageContent::Parts(responses_content_parts(
        &message.parts,
        message.role == crate::conversation::Role::Assistant,
        image_detail,
    ))
}

/// Converts one tool message body into Responses function-call output.
fn responses_tool_output(
    message: &Message,
    image_detail: ImageInputDetail,
) -> ResponsesToolOutput<'_> {
    if message.parts.is_empty() {
        return ResponsesToolOutput::Text(&message.content);
    }

    ResponsesToolOutput::Parts(responses_content_parts(&message.parts, false, image_detail))
}

/// Converts stored text/image parts into Responses content blocks.
fn responses_content_parts(
    parts: &[MessagePart],
    assistant_output: bool,
    image_detail: ImageInputDetail,
) -> Vec<ResponsesContentPart<'_>> {
    let text_kind = if assistant_output {
        "output_text"
    } else {
        "input_text"
    };

    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => ResponsesContentPart::Text(ResponsesTextPart {
                kind: text_kind,
                text,
            }),
            MessagePart::Image(image) => {
                ResponsesContentPart::Image(responses_image_part(image, image_detail))
            }
        })
        .collect()
}

/// Encodes one persisted image as the data URL accepted by Responses.
fn responses_image_part(image: &ImagePart, detail: ImageInputDetail) -> ResponsesImagePart {
    ResponsesImagePart {
        kind: "input_image",
        image_url: format!(
            "data:{};base64,{}",
            image.mime_type,
            STANDARD.encode(&image.bytes)
        ),
        detail: detail.as_wire_value(),
    }
}

/// Converts Windie's tool schemas into Responses function tool definitions.
fn responses_tools(tools: &[ToolSchema]) -> Option<Vec<ResponsesTool<'_>>> {
    if tools.is_empty() {
        return None;
    }

    Some(
        tools
            .iter()
            .map(|tool| ResponsesTool {
                kind: "function",
                name: tool.name.as_str(),
                description: tool.description.as_str(),
                parameters: &tool.parameters,
            })
            .collect(),
    )
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

/// Moves all currently valid UTF-8 text from bytes into the text buffer while
/// keeping an incomplete final character for the next network chunk.
fn append_valid_utf8(byte_buffer: &mut Vec<u8>, text_buffer: &mut String) -> Result<()> {
    match std::str::from_utf8(byte_buffer) {
        Ok(text) => {
            text_buffer.push_str(text);
            byte_buffer.clear();
            Ok(())
        }
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            if valid_up_to > 0 {
                let text = std::str::from_utf8(&byte_buffer[..valid_up_to])
                    .context("responses stream contained invalid utf-8")?
                    .to_string();
                text_buffer.push_str(&text);
                byte_buffer.drain(..valid_up_to);
            }

            if error.error_len().is_some() {
                return Err(anyhow!("responses stream contained invalid utf-8"));
            }

            Ok(())
        }
    }
}

/// Flushes remaining UTF-8 bytes after the stream ends and rejects incomplete
/// trailing characters.
fn finish_utf8(byte_buffer: &mut Vec<u8>, text_buffer: &mut String) -> Result<()> {
    append_valid_utf8(byte_buffer, text_buffer)?;

    if !byte_buffer.is_empty() {
        return Err(anyhow!("responses stream ended with incomplete utf-8"));
    }

    Ok(())
}

/// Processes every complete newline-delimited SSE line currently buffered.
fn process_stream_lines<F>(
    buffer: &mut String,
    state: &mut AssistantStreamState,
    handle_delta: &mut F,
) -> Result<()>
where
    F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
{
    while let Some(line_end) = buffer.find('\n') {
        let line = buffer[..line_end].trim_end_matches('\r').to_string();
        buffer.drain(..=line_end);
        process_stream_line(&line, state, handle_delta)?;
    }

    Ok(())
}

/// Processes one final stream line when the server closes without a trailing
/// newline.
fn process_final_stream_line<F>(
    buffer: &mut String,
    state: &mut AssistantStreamState,
    handle_delta: &mut F,
) -> Result<()>
where
    F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
{
    if buffer.trim().is_empty() {
        return Ok(());
    }

    let line = std::mem::take(buffer);
    process_stream_line(line.trim_end_matches('\r'), state, handle_delta)
}

/// Parses one SSE line and forwards assistant content deltas.
fn process_stream_line<F>(
    line: &str,
    state: &mut AssistantStreamState,
    handle_delta: &mut F,
) -> Result<()>
where
    F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
{
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(());
    };

    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }

    let event: ResponsesStreamEvent =
        serde_json::from_str(data).context("failed to parse responses stream event")?;
    state.push_event(event, handle_delta)
}

impl AssistantStreamState {
    /// Applies one Responses stream event to the accumulated assistant turn.
    fn push_event<F>(&mut self, event: ResponsesStreamEvent, handle_delta: &mut F) -> Result<()>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        match event.kind.as_str() {
            "response.output_text.delta" => {
                if let Some(delta) = event.delta {
                    self.content.push_str(&delta);
                    handle_delta(LlmStreamEvent::AssistantDelta(&delta))?;
                }
            }
            "response.output_text.done" => {
                if self.content.is_empty()
                    && let Some(text) = event.text
                {
                    self.content = text;
                }
            }
            "response.refusal.delta" => {
                if let Some(refusal) = event.refusal.or(event.delta) {
                    append_optional_text(&mut self.metadata.refusal, &refusal);
                }
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(reasoning) = event.delta {
                    append_optional_text(&mut self.metadata.reasoning, &reasoning);
                    handle_delta(LlmStreamEvent::ReasoningDelta(&reasoning))?;
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = event.delta {
                    let key = self.partial_tool_call_key(event.output_index);
                    self.tool_calls
                        .entry(key)
                        .or_default()
                        .arguments
                        .push_str(&delta);
                    let (id, name) = self.tool_call_snapshot(key);
                    handle_delta(LlmStreamEvent::ToolCallDelta {
                        index: key,
                        id,
                        name,
                        arguments_delta: Some(&delta),
                    })?;
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = event.arguments {
                    self.partial_tool_call(event.output_index).arguments = arguments;
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                if let Some(item) = event.item
                    && let Some(key) = self.push_output_item(event.output_index, item)
                {
                    let (id, name) = self.tool_call_snapshot(key);
                    handle_delta(LlmStreamEvent::ToolCallDelta {
                        index: key,
                        id,
                        name,
                        arguments_delta: None,
                    })?;
                }
            }
            "response.completed" => {
                self.push_response_usage(event.response);
                self.finish_reason.get_or_insert(FinishReason::Stop);
            }
            "response.incomplete" => {
                self.push_response_usage(event.response);
                self.finish_reason = Some(FinishReason::Length);
            }
            "response.failed" | "error" => {
                return Err(anyhow!(
                    "responses stream failed: {}",
                    event_error_text(&event)
                ));
            }
            _ => {}
        }

        Ok(())
    }

    /// Applies a streamed function-call item.
    fn push_output_item(
        &mut self,
        output_index: Option<u16>,
        item: ResponsesStreamItem,
    ) -> Option<u16> {
        if item.kind.as_deref() != Some("function_call") {
            return None;
        }

        let key = self.tool_call_key(output_index, item.call_id.as_deref().or(item.id.as_deref()));
        let partial = self.tool_calls.entry(key).or_default();

        if item.call_id.is_some() {
            partial.id = item.call_id;
        } else if item.id.is_some() {
            partial.id = item.id;
        }
        if let Some(name) = item.name {
            partial.name = Some(name);
        }
        if let Some(arguments) = item.arguments {
            partial.arguments = arguments;
        }
        self.finish_reason = Some(FinishReason::ToolCalls);
        Some(key)
    }

    /// Persists provider-reported usage from terminal response events.
    fn push_response_usage(&mut self, response: Option<ResponsesStreamResponse>) {
        let Some(usage) = response.and_then(|response| response.usage) else {
            return;
        };

        self.metadata.usage = Some(token_usage_from_raw(usage));
    }

    /// Returns the mutable partial tool call for one stream output index.
    fn partial_tool_call(&mut self, output_index: Option<u16>) -> &mut PartialToolCall {
        let key = self.partial_tool_call_key(output_index);
        self.tool_calls.entry(key).or_default()
    }

    /// Returns the stream key for argument-only function-call events.
    fn partial_tool_call_key(&self, output_index: Option<u16>) -> u16 {
        output_index.unwrap_or_else(|| self.next_tool_call_key())
    }

    /// Returns current function-call identifiers for one stream key.
    fn tool_call_snapshot(&self, key: u16) -> (Option<&str>, Option<&str>) {
        self.tool_calls
            .get(&key)
            .map(|partial| (partial.id.as_deref(), partial.name.as_deref()))
            .unwrap_or((None, None))
    }

    /// Finds the stream key for a function-call item.
    fn tool_call_key(&self, output_index: Option<u16>, tool_call_id: Option<&str>) -> u16 {
        if let Some(output_index) = output_index {
            return output_index;
        }
        if let Some(tool_call_id) = tool_call_id
            && let Some((key, _)) = self
                .tool_calls
                .iter()
                .find(|(_, partial)| partial.id.as_deref() == Some(tool_call_id))
        {
            return *key;
        }

        self.next_tool_call_key()
    }

    /// Returns a stream key greater than any existing partial tool call key.
    fn next_tool_call_key(&self) -> u16 {
        self.tool_calls
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Converts the stream state into a complete assistant response.
    fn finalize(self) -> Result<AssistantResponse> {
        let mut metadata = self.metadata;
        let tool_calls = self
            .tool_calls
            .into_values()
            .enumerate()
            .map(|(index, tool_call)| tool_call.finalize(index))
            .collect::<Result<Vec<_>>>()?;
        metadata.tool_calls = tool_calls;

        Ok(AssistantResponse {
            content: self.content,
            metadata,
            finish_reason: self.finish_reason,
        })
    }
}

impl PartialToolCall {
    /// Validates and returns one complete tool call after streaming has ended.
    fn finalize(self, index: usize) -> Result<ToolCall> {
        let id = self
            .id
            .ok_or_else(|| anyhow!("tool call {index} did not include id"))?;
        let name = self
            .name
            .ok_or_else(|| anyhow!("tool call {index} did not include function name"))?;

        let mut tool_call = ToolCall::function(id, name, self.arguments);
        tool_call.index = u16::try_from(index).unwrap_or(u16::MAX);

        Ok(tool_call)
    }
}

/// Extracts stable token totals while preserving the full raw usage payload.
fn token_usage_from_raw(raw: serde_json::Value) -> TokenUsage {
    TokenUsage {
        input_tokens: raw.get("input_tokens").and_then(serde_json::Value::as_u64),
        output_tokens: raw.get("output_tokens").and_then(serde_json::Value::as_u64),
        total_tokens: raw.get("total_tokens").and_then(serde_json::Value::as_u64),
        raw,
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

/// Returns the most useful error text from a failed Responses stream event.
fn event_error_text(event: &ResponsesStreamEvent) -> String {
    event
        .error
        .as_ref()
        .and_then(|error| error.message.as_deref())
        .or(event.message.as_deref())
        .or_else(|| {
            event
                .response
                .as_ref()
                .and_then(|response| response.error.as_ref())
                .and_then(|error| error.message.as_deref())
        })
        .or_else(|| event.error.as_ref().and_then(|error| error.code.as_deref()))
        .or_else(|| event.error.as_ref().and_then(|error| error.kind.as_deref()))
        .unwrap_or("unknown responses stream error")
        .to_string()
}

/// Appends one text delta into an optional accumulated text field.
fn append_optional_text(target: &mut Option<String>, delta: &str) {
    match target {
        Some(value) => value.push_str(delta),
        None => *target = Some(delta.to_string()),
    }
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
