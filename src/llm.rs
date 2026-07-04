//! OpenAI-compatible streaming LLM client.
//!
//! This module owns OpenAI-compatible request serialization, HTTP requests to
//! Bifrost's chat completions endpoint, and streamed response parsing.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::conversation::{
    AssistantAnnotation, AssistantAudio, ImagePart, Message, MessageMetadata, MessagePart,
    ReasoningDetail, ToolCall, ToolSchema,
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
/// Complete assistant response received from a streaming chat request.
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
    Other,
}

/// HTTP client for Bifrost's OpenAI-compatible chat completions endpoint.
pub struct BifrostClient {
    http: Client,
    base_url: BaseUrl,
    model: ModelName,
}

#[derive(Debug, Serialize)]
/// JSON request body sent to `/chat/completions`.
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatTool<'a>>>,
    stream: bool,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible message payload sent to Bifrost.
struct ChatMessage<'a> {
    role: crate::conversation::Role,
    content: ChatMessageContent<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<&'a [ToolCall]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_details: Option<&'a [ReasoningDetail]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<&'a AssistantAudio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<&'a [AssistantAnnotation]>,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible function tool definition sent to Bifrost.
struct ChatTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatToolFunction<'a>,
}

#[derive(Debug, Serialize)]
/// Function schema inside one OpenAI-compatible tool definition.
struct ChatToolFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// OpenAI-compatible content field: plain text or ordered multimodal parts.
enum ChatMessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ChatContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One OpenAI-compatible multimodal content part.
enum ChatContentPart<'a> {
    Text(ChatTextPart<'a>),
    Image(ChatImagePart),
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible text content part.
struct ChatTextPart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible image content part.
struct ChatImagePart {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: ChatImageUrl,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible image URL payload.
struct ChatImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
/// One streamed server-sent event payload from the provider adapter.
struct ChatStreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
/// One candidate choice inside a streamed chat chunk.
struct StreamChoice {
    finish_reason: Option<String>,
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
/// Incremental assistant content inside a streamed choice.
struct StreamDelta {
    content: Option<String>,
    refusal: Option<String>,
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_details: Vec<ReasoningDetail>,
    audio: Option<AssistantAudio>,
    #[serde(default)]
    annotations: Vec<AssistantAnnotation>,
    #[serde(default)]
    tool_calls: Vec<StreamToolCallDelta>,
}

#[derive(Debug, Deserialize)]
/// Incremental tool-call payload inside a streamed choice.
struct StreamToolCallDelta {
    index: u16,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<StreamToolCallFunctionDelta>,
}

#[derive(Debug, Deserialize)]
/// Incremental function-call fields inside a streamed tool call.
struct StreamToolCallFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Default)]
/// In-progress assistant stream state.
struct AssistantStreamState {
    content: String,
    metadata: MessageMetadata,
    tool_calls: BTreeMap<u16, PartialToolCall>,
    reasoning_details: BTreeMap<u16, PartialReasoningDetail>,
    finish_reason: Option<FinishReason>,
}

#[derive(Debug, Default)]
/// Tool call assembled from multiple streaming chunks.
struct PartialToolCall {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug)]
/// Reasoning detail assembled from multiple streaming chunks.
struct PartialReasoningDetail {
    detail: ReasoningDetail,
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

    /// Builds the chat completions endpoint from the normalized base URL.
    pub fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Sends the chat request, streams assistant text deltas to the caller, and
    /// returns the complete assistant response including tool calls.
    pub async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let request = ChatRequest {
            model: self.model.as_str(),
            messages: chat_messages(messages),
            tools: chat_tools(tools),
            stream: true,
        };

        let response = self
            .http
            .post(self.chat_endpoint())
            .json(&request)
            .send()
            .await
            .context("failed to send chat request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("chat request failed with {status}: {body}"));
        }

        let mut stream = response.bytes_stream();
        let mut byte_buffer = Vec::new();
        let mut buffer = String::new();
        let mut state = AssistantStreamState::default();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read chat stream")?;

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
                "chat stream did not include assistant content or metadata"
            ));
        }

        Ok(response)
    }
}

/// Converts Windie's internal messages into the OpenAI-compatible request shape.
fn chat_messages(messages: &[Message]) -> Vec<ChatMessage<'_>> {
    messages.iter().map(ChatMessage::from_message).collect()
}

impl<'a> ChatMessage<'a> {
    /// Builds one provider request message from the runtime message model.
    fn from_message(message: &'a Message) -> Self {
        let tool_calls = message
            .metadata
            .as_ref()
            .filter(|_| message.role == crate::conversation::Role::Assistant)
            .map(|metadata| metadata.tool_calls.as_slice())
            .filter(|tool_calls| !tool_calls.is_empty());
        let metadata = message
            .metadata
            .as_ref()
            .filter(|_| message.role == crate::conversation::Role::Assistant);

        Self {
            role: message.role,
            content: chat_message_content(message),
            tool_calls,
            refusal: metadata.and_then(|metadata| metadata.refusal.as_deref()),
            reasoning: metadata.and_then(|metadata| metadata.reasoning.as_deref()),
            reasoning_details: metadata
                .map(|metadata| metadata.reasoning_details.as_slice())
                .filter(|reasoning_details| !reasoning_details.is_empty()),
            audio: metadata.and_then(|metadata| metadata.audio.as_ref()),
            annotations: metadata
                .map(|metadata| metadata.annotations.as_slice())
                .filter(|annotations| !annotations.is_empty()),
        }
    }
}

/// Converts Windie's tool schemas into the OpenAI-compatible request shape.
fn chat_tools(tools: &[ToolSchema]) -> Option<Vec<ChatTool<'_>>> {
    if tools.is_empty() {
        return None;
    }

    Some(
        tools
            .iter()
            .map(|tool| ChatTool {
                kind: "function",
                function: ChatToolFunction {
                    name: tool.name.as_str(),
                    description: tool.description.as_str(),
                    parameters: &tool.parameters,
                },
            })
            .collect(),
    )
}

/// Converts one message body into plain text or ordered multimodal parts.
fn chat_message_content(message: &Message) -> ChatMessageContent<'_> {
    if message.parts.is_empty() {
        return ChatMessageContent::Text(&message.content);
    }

    ChatMessageContent::Parts(chat_content_parts(&message.parts))
}

/// Converts stored text/image parts into OpenAI-compatible content parts.
fn chat_content_parts(parts: &[MessagePart]) -> Vec<ChatContentPart<'_>> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => ChatContentPart::Text(ChatTextPart { kind: "text", text }),
            MessagePart::Image(image) => ChatContentPart::Image(chat_image_part(image)),
        })
        .collect()
}

/// Encodes one persisted image as the data URL accepted by the chat request.
fn chat_image_part(image: &ImagePart) -> ChatImagePart {
    ChatImagePart {
        kind: "image_url",
        image_url: ChatImageUrl {
            url: format!(
                "data:{};base64,{}",
                image.mime_type,
                STANDARD.encode(&image.bytes)
            ),
        },
    }
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
                    .context("chat stream contained invalid utf-8")?
                    .to_string();
                text_buffer.push_str(&text);
                byte_buffer.drain(..valid_up_to);
            }

            if error.error_len().is_some() {
                return Err(anyhow!("chat stream contained invalid utf-8"));
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
        return Err(anyhow!("chat stream ended with incomplete utf-8"));
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

    let chunk: ChatStreamChunk =
        serde_json::from_str(data).context("failed to parse chat stream chunk")?;
    for choice in chunk.choices {
        if let Some(finish_reason) = choice.finish_reason {
            state.finish_reason = Some(FinishReason::from_provider(&finish_reason));
        }

        if let Some(content) = choice.delta.content {
            handle_delta(&content)?;
            state.content.push_str(&content);
        }
        if let Some(refusal) = choice.delta.refusal {
            append_optional_text(&mut state.metadata.refusal, &refusal);
        }
        if let Some(reasoning) = choice.delta.reasoning {
            append_optional_text(&mut state.metadata.reasoning, &reasoning);
        }
        if let Some(audio) = choice.delta.audio {
            state.push_audio_delta(audio);
        }
        if !choice.delta.annotations.is_empty() {
            state.metadata.annotations.extend(choice.delta.annotations);
        }
        for reasoning_detail in choice.delta.reasoning_details {
            state.push_reasoning_detail_delta(reasoning_detail);
        }

        for tool_call in choice.delta.tool_calls {
            state.push_tool_call_delta(tool_call);
        }
    }

    Ok(())
}

impl FinishReason {
    /// Converts provider finish reason text into Windie's small enum.
    fn from_provider(value: &str) -> Self {
        match value {
            "stop" => Self::Stop,
            "length" => Self::Length,
            "tool_calls" => Self::ToolCalls,
            _ => Self::Other,
        }
    }
}

impl AssistantStreamState {
    /// Applies one streamed tool-call delta to the in-progress call at its
    /// provider index.
    fn push_tool_call_delta(&mut self, delta: StreamToolCallDelta) {
        let tool_call = self.tool_calls.entry(delta.index).or_default();

        if let Some(id) = delta.id {
            tool_call.id = Some(id);
        }
        if let Some(kind) = delta.kind {
            tool_call.kind = Some(kind);
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                tool_call.name = Some(name);
            }
            if let Some(arguments) = function.arguments {
                tool_call.arguments.push_str(&arguments);
            }
        }
    }

    /// Applies one streamed reasoning detail delta by its provider index.
    fn push_reasoning_detail_delta(&mut self, detail: ReasoningDetail) {
        self.reasoning_details
            .entry(detail.index)
            .and_modify(|partial| partial.push_delta(detail.clone()))
            .or_insert_with(|| PartialReasoningDetail { detail });
    }

    /// Applies one streamed audio delta to the assistant audio metadata lane.
    fn push_audio_delta(&mut self, delta: AssistantAudio) {
        if let Some(audio) = self.metadata.audio.as_mut() {
            if !delta.id.is_empty() {
                audio.id = delta.id;
            }
            audio.data.push_str(&delta.data);
            if delta.expires_at != 0 {
                audio.expires_at = delta.expires_at;
            }
            audio.transcript.push_str(&delta.transcript);
        } else {
            self.metadata.audio = Some(delta);
        }
    }

    /// Converts the stream state into a complete assistant response.
    fn finalize(self) -> Result<AssistantResponse> {
        let mut metadata = self.metadata;
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, tool_call)| tool_call.finalize(index))
            .collect::<Result<Vec<_>>>()?;
        metadata.tool_calls = tool_calls;
        metadata.reasoning_details = self
            .reasoning_details
            .into_values()
            .map(|partial| partial.detail)
            .collect();

        Ok(AssistantResponse {
            content: self.content,
            metadata,
            finish_reason: self.finish_reason,
        })
    }
}

impl PartialToolCall {
    /// Validates and returns one complete tool call after streaming has ended.
    fn finalize(self, index: u16) -> Result<ToolCall> {
        let id = self
            .id
            .ok_or_else(|| anyhow!("tool call {index} did not include id"))?;
        let name = self
            .name
            .ok_or_else(|| anyhow!("tool call {index} did not include function name"))?;

        let mut tool_call = ToolCall::function(id, name, self.arguments);
        tool_call.index = index;

        Ok(tool_call)
    }
}

impl PartialReasoningDetail {
    /// Appends text-like reasoning fields while keeping latest identity fields.
    fn push_delta(&mut self, delta: ReasoningDetail) {
        self.detail.kind = delta.kind;
        if delta.id.is_some() {
            self.detail.id = delta.id;
        }
        append_optional_field(&mut self.detail.summary, delta.summary);
        append_optional_field(&mut self.detail.text, delta.text);
        if delta.signature.is_some() {
            self.detail.signature = delta.signature;
        }
        append_optional_field(&mut self.detail.data, delta.data);
    }
}

/// Appends one text delta into an optional accumulated text field.
fn append_optional_text(target: &mut Option<String>, delta: &str) {
    match target {
        Some(value) => value.push_str(delta),
        None => *target = Some(delta.to_string()),
    }
}

/// Appends an optional text delta into an optional accumulated text field.
fn append_optional_field(target: &mut Option<String>, delta: Option<String>) {
    if let Some(delta) = delta {
        append_optional_text(target, &delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{
        ImageAssetId, MessageId, MessageMetadata, ReasoningDetailKind, Role, ToolCallFunction,
        ToolCallId, ToolCallKind, ToolSchema, ToolSchemaName,
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
    fn builds_chat_endpoint_from_base_url() {
        let llm = BifrostClient::new(
            BaseUrl::new("http://localhost:8080/v1/"),
            ModelName::new("openai/gpt-4o-mini"),
        );

        assert_eq!(
            llm.chat_endpoint(),
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn serializes_text_message_for_chat_request() {
        let messages = vec![Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::User,
            content: "hello".to_string(),
            parts: Vec::new(),
            metadata: None,
        }];
        let request = ChatRequest {
            model: "openai/gpt-4o-mini",
            messages: chat_messages(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_assistant_tool_calls_for_chat_request() {
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
        let request = ChatRequest {
            model: "openai/gpt-4o-mini",
            messages: chat_messages(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "messages": [{
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call-id",
                        "type": "function",
                        "function": {
                            "name": "run_shell",
                            "arguments": "{\"command\":\"ls\"}"
                        }
                    }]
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_user_image_parts_for_chat_request() {
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
        let request = ChatRequest {
            model: "openai/gpt-4o-mini",
            messages: chat_messages(&messages),
            tools: None,
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "what is this?"},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,AQID"}}
                    ]
                }],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_tool_schemas_for_chat_request() {
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
        let request = ChatRequest {
            model: "openai/gpt-4o-mini",
            messages: Vec::new(),
            tools: chat_tools(&tools),
            stream: true,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "messages": [],
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "run_shell",
                        "description": "Run a shell command",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "command": {"type": "string"}
                            },
                            "required": ["command"]
                        }
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
            r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

        assert_eq!(state.content, "Hello");
        assert_eq!(deltas, vec!["Hello"]);
    }

    #[test]
    fn parses_stream_assistant_metadata_lanes() {
        let mut state = AssistantStreamState::default();
        let mut handle_delta = |_text: &str| Ok(());

        process_stream_line(
            r#"data: {"choices":[{"delta":{"refusal":"no","reasoning":"think","reasoning_details":[{"index":0,"type":"reasoning.text","text":"think"}]}}]}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

        let response = state.finalize().unwrap();

        assert_eq!(response.metadata.refusal.as_deref(), Some("no"));
        assert_eq!(response.metadata.reasoning.as_deref(), Some("think"));
        assert_eq!(response.metadata.reasoning_details.len(), 1);
        assert_eq!(
            response.metadata.reasoning_details[0].kind,
            ReasoningDetailKind::Text
        );
        assert_eq!(
            response.metadata.reasoning_details[0].text.as_deref(),
            Some("think")
        );
    }

    #[test]
    fn assembles_streamed_tool_call() {
        let mut state = AssistantStreamState::default();
        let mut handle_delta = |_text: &str| Ok(());

        process_stream_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_123","type":"function","function":{"name":"run_shell","arguments":"{\"command\""}}]}}]}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();
        process_stream_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"ls\"}"}}]},"finish_reason":"tool_calls"}]}"#,
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

        assert_eq!(error.to_string(), "chat stream contained invalid utf-8");
    }

    #[test]
    fn rejects_incomplete_final_utf8_bytes() {
        let mut byte_buffer = vec![0xe4];
        let mut text_buffer = String::new();

        let error = finish_utf8(&mut byte_buffer, &mut text_buffer).unwrap_err();

        assert_eq!(error.to_string(), "chat stream ended with incomplete utf-8");
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
    fn ignores_non_data_stream_line() {
        let mut state = AssistantStreamState::default();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_line("event: message", &mut state, &mut handle_delta).unwrap();

        assert!(state.content.is_empty());
        assert!(deltas.is_empty());
    }

    #[test]
    fn accumulates_multiple_stream_lines() {
        let mut buffer = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\
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

    #[test]
    fn keeps_partial_line_in_buffer() {
        let mut buffer = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}".to_string();
        let mut state = AssistantStreamState::default();
        let mut deltas = Vec::new();

        {
            let mut handle_delta = |text: &str| {
                deltas.push(text.to_string());
                Ok(())
            };

            process_stream_lines(&mut buffer, &mut state, &mut handle_delta).unwrap();
        }

        assert!(!buffer.is_empty());
        assert!(state.content.is_empty());
        assert!(deltas.is_empty());

        {
            let mut handle_delta = |text: &str| {
                deltas.push(text.to_string());
                Ok(())
            };

            process_final_stream_line(&mut buffer, &mut state, &mut handle_delta).unwrap();
        }

        assert!(buffer.is_empty());
        assert_eq!(state.content, "Hello");
        assert_eq!(deltas, vec!["Hello"]);
    }
}
