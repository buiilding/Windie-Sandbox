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
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::conversation::{ImagePart, Message, MessageMetadata, MessagePart, ToolCall, ToolSchema};

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
        handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq)]
/// Complete assistant response received from a streaming Responses request.
pub struct AssistantResponse {
    pub content: String,
    pub metadata: MessageMetadata,
    pub finish_reason: Option<FinishReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Normalized reason the provider stopped the assistant stream.
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
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
    stream: bool,
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

    /// Sends the Responses request, streams assistant text deltas to the
    /// caller, and returns the complete assistant response including tool
    /// calls.
    pub async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let request = ResponsesRequest {
            model: self.model.as_str(),
            input: responses_input(messages),
            tools: responses_tools(tools),
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

/// Converts Windie's internal messages into the Responses request input array.
fn responses_input(messages: &[Message]) -> Vec<ResponsesInputItem<'_>> {
    messages
        .iter()
        .flat_map(responses_items_for_message)
        .collect()
}

/// Converts one Windie message into one or more Responses input items.
fn responses_items_for_message(message: &Message) -> Vec<ResponsesInputItem<'_>> {
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
                        content: responses_message_content(message),
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
                content: responses_message_content(message),
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
                            output: responses_tool_output(message),
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
                content: responses_message_content(message),
            })]
        }
        crate::conversation::Role::User => {
            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "user",
                content: responses_message_content(message),
            })]
        }
    }
}

/// Converts one normal message body into Responses content.
fn responses_message_content(message: &Message) -> ResponsesMessageContent<'_> {
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
    ))
}

/// Converts one tool message body into Responses function-call output.
fn responses_tool_output(message: &Message) -> ResponsesToolOutput<'_> {
    if message.parts.is_empty() {
        return ResponsesToolOutput::Text(&message.content);
    }

    ResponsesToolOutput::Parts(responses_content_parts(&message.parts, false))
}

/// Converts stored text/image parts into Responses content blocks.
fn responses_content_parts(
    parts: &[MessagePart],
    assistant_output: bool,
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
            MessagePart::Image(image) => ResponsesContentPart::Image(responses_image_part(image)),
        })
        .collect()
}

/// Encodes one persisted image as the data URL accepted by Responses.
fn responses_image_part(image: &ImagePart) -> ResponsesImagePart {
    ResponsesImagePart {
        kind: "input_image",
        image_url: format!(
            "data:{};base64,{}",
            image.mime_type,
            STANDARD.encode(&image.bytes)
        ),
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
        handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        BifrostClient::stream(self, messages, tools, handle_delta).await
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
    F: FnMut(&str) -> Result<()>,
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
    F: FnMut(&str) -> Result<()>,
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
    F: FnMut(&str) -> Result<()>,
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
        F: FnMut(&str) -> Result<()>,
    {
        match event.kind.as_str() {
            "response.output_text.delta" => {
                if let Some(delta) = event.delta {
                    handle_delta(&delta)?;
                    self.content.push_str(&delta);
                }
            }
            "response.output_text.done" => {
                if self.content.is_empty() {
                    if let Some(text) = event.text {
                        self.content = text;
                    }
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
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = event.delta {
                    self.partial_tool_call(event.output_index)
                        .arguments
                        .push_str(&delta);
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = event.arguments {
                    self.partial_tool_call(event.output_index).arguments = arguments;
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                if let Some(item) = event.item {
                    self.push_output_item(event.output_index, item);
                }
            }
            "response.completed" => {
                self.finish_reason.get_or_insert(FinishReason::Stop);
            }
            "response.incomplete" => {
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
    fn push_output_item(&mut self, output_index: Option<u16>, item: ResponsesStreamItem) {
        if item.kind.as_deref() != Some("function_call") {
            return;
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
    }

    /// Returns the mutable partial tool call for one stream output index.
    fn partial_tool_call(&mut self, output_index: Option<u16>) -> &mut PartialToolCall {
        let key = output_index.unwrap_or_else(|| self.next_tool_call_key());
        self.tool_calls.entry(key).or_default()
    }

    /// Finds the stream key for a function-call item.
    fn tool_call_key(&self, output_index: Option<u16>, tool_call_id: Option<&str>) -> u16 {
        if let Some(output_index) = output_index {
            return output_index;
        }
        if let Some(tool_call_id) = tool_call_id {
            if let Some((key, _)) = self
                .tool_calls
                .iter()
                .find(|(_, partial)| partial.id.as_deref() == Some(tool_call_id))
            {
                return *key;
            }
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
mod tests {
    use super::*;
    use crate::conversation::{
        ImageAssetId, MessageId, MessageMetadata, Role, ToolCallFunction, ToolCallId, ToolCallKind,
        ToolSchema, ToolSchemaName,
    };

    #[test]
    fn base_url_removes_trailing_slash() {
        let base_url = BaseUrl::new("http://localhost:8080/v1/");

        assert_eq!(base_url.as_str(), "http://localhost:8080/v1");
    }

    #[test]
    fn model_name_preserves_provider_prefix() {
        let model = ModelName::new("anthropic/claude-3-5-haiku");

        assert_eq!(model.as_str(), "anthropic/claude-3-5-haiku");
    }

    #[test]
    fn builds_responses_endpoint_from_base_url() {
        let llm = BifrostClient::new(
            BaseUrl::new("http://localhost:8080/v1/"),
            ModelName::new("openai/gpt-4o-mini"),
        );

        assert_eq!(
            llm.responses_endpoint(),
            "http://localhost:8080/v1/responses"
        );
    }

    #[test]
    fn builds_models_endpoint_from_base_url() {
        let base_url = BaseUrl::new("http://localhost:8080/v1/");

        assert_eq!(
            models_endpoint(&base_url),
            "http://localhost:8080/v1/models"
        );
    }

    #[test]
    fn serializes_text_message_for_responses_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::User,
            content: "hello".to_string(),
            parts: Vec::new(),
            metadata: None,
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: responses_input(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [{"type": "message", "role": "user", "content": "hello"}],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_assistant_tool_calls_for_responses_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::Assistant,
            content: String::new(),
            parts: Vec::new(),
            metadata: Some(MessageMetadata {
                tool_calls: vec![ToolCall {
                    index: 0,
                    id: ToolCallId::new("call-id"),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "run_shell".to_string(),
                        arguments: r#"{"command":"ls"}"#.to_string(),
                    },
                }],
                ..Default::default()
            }),
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: responses_input(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [{
                    "type": "function_call",
                    "call_id": "call-id",
                    "name": "run_shell",
                    "arguments": "{\"command\":\"ls\"}",
                    "status": "completed"
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_assistant_text_before_tool_call_for_responses_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::Assistant,
            content: "I will inspect the desktop.".to_string(),
            parts: Vec::new(),
            metadata: Some(MessageMetadata {
                tool_calls: vec![ToolCall {
                    index: 0,
                    id: ToolCallId::new("call-id"),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "cua_driver__get_desktop_state".to_string(),
                        arguments: "{}".to_string(),
                    },
                }],
                ..Default::default()
            }),
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: responses_input(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": "I will inspect the desktop."
                        }]
                    },
                    {
                        "type": "function_call",
                        "call_id": "call-id",
                        "name": "cua_driver__get_desktop_state",
                        "arguments": "{}",
                        "status": "completed"
                    }
                ],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_user_image_parts_for_responses_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: None,
            role: Role::User,
            content: "what is this?".to_string(),
            parts: vec![
                MessagePart::Text("what is this?".to_string()),
                MessagePart::Image(ImagePart {
                    asset_id: ImageAssetId::new("image-id"),
                    mime_type: "image/png".to_string(),
                    bytes: vec![1, 2, 3],
                }),
            ],
            metadata: None,
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: responses_input(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "what is this?"},
                        {"type": "input_image", "image_url": "data:image/png;base64,AQID"}
                    ]
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_tool_message_call_id_for_responses_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::Tool,
            content: r#"{"stdout":"ok"}"#.to_string(),
            parts: Vec::new(),
            metadata: Some(MessageMetadata {
                tool_call_id: Some(ToolCallId::new("call-id")),
                ..Default::default()
            }),
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: responses_input(&messages),
            tools: None,
            stream: true,
        };

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [{
                    "type": "function_call_output",
                    "call_id": "call-id",
                    "output": "{\"stdout\":\"ok\"}",
                    "status": "completed"
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_tool_image_parts_for_responses_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::Tool,
            content: "screenshot".to_string(),
            parts: vec![
                MessagePart::Text("screenshot".to_string()),
                MessagePart::Image(ImagePart {
                    asset_id: ImageAssetId::new("image-id"),
                    mime_type: "image/png".to_string(),
                    bytes: vec![1, 2, 3],
                }),
            ],
            metadata: Some(MessageMetadata {
                tool_call_id: Some(ToolCallId::new("call-id")),
                ..Default::default()
            }),
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: responses_input(&messages),
            tools: None,
            stream: true,
        };

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [{
                    "type": "function_call_output",
                    "call_id": "call-id",
                    "output": [
                        {"type": "input_text", "text": "screenshot"},
                        {"type": "input_image", "image_url": "data:image/png;base64,AQID"}
                    ],
                    "status": "completed"
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_tool_schemas_for_responses_request() {
        let tools = vec![ToolSchema {
            name: ToolSchemaName::new("run_shell"),
            description: "Run a shell command".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }),
        }];
        let request = ResponsesRequest {
            model: "openai/gpt-4o-mini",
            input: Vec::new(),
            tools: responses_tools(&tools),
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "input": [],
                "tools": [{
                    "type": "function",
                    "name": "run_shell",
                    "description": "Run a shell command",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "command": {"type": "string"}
                        },
                        "required": ["command"]
                    }
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn parses_stream_content_delta() {
        let mut state = AssistantStreamState::default();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_line(
            r#"data: {"type":"response.output_text.delta","delta":"Hello"}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

        assert_eq!(state.content, "Hello");
        assert_eq!(deltas, vec!["Hello"]);
    }

    #[test]
    fn parses_stream_metadata_lanes() {
        let mut state = AssistantStreamState::default();
        let mut handle_delta = |_text: &str| Ok(());

        process_stream_line(
            r#"data: {"type":"response.refusal.delta","refusal":"no"}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();
        process_stream_line(
            r#"data: {"type":"response.reasoning_summary_text.delta","delta":"think"}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

        let response = state.finalize().unwrap();

        assert_eq!(response.metadata.refusal.as_deref(), Some("no"));
        assert_eq!(response.metadata.reasoning.as_deref(), Some("think"));
    }

    #[test]
    fn assembles_streamed_tool_call() {
        let mut state = AssistantStreamState::default();
        let mut handle_delta = |_text: &str| Ok(());

        process_stream_line(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_123","name":"run_shell","arguments":""}}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();
        process_stream_line(
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"command\""}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();
        process_stream_line(
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":":\"ls\"}"}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

        let response = state.finalize().unwrap();

        assert_eq!(response.finish_reason, Some(FinishReason::ToolCalls));
        assert_eq!(response.metadata.tool_calls.len(), 1);
        assert_eq!(response.metadata.tool_calls[0].id.as_str(), "call_123");
        assert_eq!(response.metadata.tool_calls[0].name(), "run_shell");
        assert_eq!(
            response.metadata.tool_calls[0].arguments(),
            r#"{"command":"ls"}"#
        );
    }

    #[test]
    fn buffers_split_utf8_bytes() {
        let text = "你";
        let bytes = text.as_bytes();
        let mut byte_buffer = Vec::new();
        let mut text_buffer = String::new();

        byte_buffer.extend_from_slice(&bytes[..1]);
        append_valid_utf8(&mut byte_buffer, &mut text_buffer).unwrap();

        assert!(text_buffer.is_empty());
        assert_eq!(byte_buffer, bytes[..1]);

        byte_buffer.extend_from_slice(&bytes[1..]);
        append_valid_utf8(&mut byte_buffer, &mut text_buffer).unwrap();

        assert_eq!(text_buffer, text);
        assert!(byte_buffer.is_empty());
    }

    #[test]
    fn rejects_invalid_utf8_bytes() {
        let mut byte_buffer = vec![0xff];
        let mut text_buffer = String::new();

        let error = append_valid_utf8(&mut byte_buffer, &mut text_buffer).unwrap_err();

        assert_eq!(
            error.to_string(),
            "responses stream contained invalid utf-8"
        );
    }

    #[test]
    fn rejects_incomplete_final_utf8_bytes() {
        let mut byte_buffer = vec![0xe4];
        let mut text_buffer = String::new();

        let error = finish_utf8(&mut byte_buffer, &mut text_buffer).unwrap_err();

        assert_eq!(
            error.to_string(),
            "responses stream ended with incomplete utf-8"
        );
    }

    #[test]
    fn ignores_done_stream_line() {
        let mut state = AssistantStreamState::default();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_line("data: [DONE]", &mut state, &mut handle_delta).unwrap();

        assert!(state.content.is_empty());
        assert!(deltas.is_empty());
    }

    #[test]
    fn accumulates_multiple_stream_lines() {
        let mut buffer = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\
             data: [DONE]\n"
            .to_string();
        let mut state = AssistantStreamState::default();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_lines(&mut buffer, &mut state, &mut handle_delta).unwrap();

        assert!(buffer.is_empty());
        assert_eq!(state.content, "Hello");
        assert_eq!(deltas, vec!["Hel", "lo"]);
    }
}
