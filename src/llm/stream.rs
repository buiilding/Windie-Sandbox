//! Responses stream parsing and assistant response assembly.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};

use crate::conversation::{MessageMetadata, TokenUsage, ToolCall};

use super::responses::{ResponsesStreamEvent, ResponsesStreamItem, ResponsesStreamResponse};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Normalized reason the provider stopped the assistant stream.
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
}
#[derive(Debug, Default)]
/// In-progress assistant stream state.
pub(super) struct AssistantStreamState {
    pub(super) content: String,
    pub(super) metadata: MessageMetadata,
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
/// Moves all currently valid UTF-8 text from bytes into the text buffer while
/// keeping an incomplete final character for the next network chunk.
pub(super) fn append_valid_utf8(byte_buffer: &mut Vec<u8>, text_buffer: &mut String) -> Result<()> {
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
pub(super) fn finish_utf8(byte_buffer: &mut Vec<u8>, text_buffer: &mut String) -> Result<()> {
    append_valid_utf8(byte_buffer, text_buffer)?;

    if !byte_buffer.is_empty() {
        return Err(anyhow!("responses stream ended with incomplete utf-8"));
    }

    Ok(())
}

/// Processes every complete newline-delimited SSE line currently buffered.
pub(super) fn process_stream_lines<F>(
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
pub(super) fn process_final_stream_line<F>(
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
pub(super) fn process_stream_line<F>(
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
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(reasoning) = event.delta {
                    append_optional_text(&mut self.metadata.reasoning, &reasoning);
                    handle_delta(LlmStreamEvent::ReasoningDelta(&reasoning))?;
                }
            }
            "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
                if self.metadata.reasoning.is_none()
                    && let Some(reasoning) = event.text
                {
                    self.metadata.reasoning = Some(reasoning);
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
    pub(super) fn finalize(self) -> Result<AssistantResponse> {
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
pub(super) fn input_token_count_from_raw(raw: serde_json::Value) -> Result<InputTokenCount> {
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
