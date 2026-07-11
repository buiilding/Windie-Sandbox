//! SSE decoding and streamed assistant response accumulation.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use super::*;
use crate::conversation::{TokenUsage, ToolCall};

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

/// Incrementally decodes network chunks into one complete assistant response.
pub(super) struct ResponseStreamDecoder {
    byte_buffer: Vec<u8>,
    text_buffer: String,
    state: AssistantStreamState,
}

impl ResponseStreamDecoder {
    pub(super) fn new() -> Self {
        Self {
            byte_buffer: Vec::new(),
            text_buffer: String::new(),
            state: AssistantStreamState::default(),
        }
    }

    pub(super) fn push<F>(&mut self, chunk: &[u8], handle_delta: &mut F) -> Result<()>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        self.byte_buffer.extend_from_slice(chunk);
        append_valid_utf8(&mut self.byte_buffer, &mut self.text_buffer)?;
        process_stream_lines(&mut self.text_buffer, &mut self.state, handle_delta)
    }

    pub(super) fn finish<F>(mut self, handle_delta: &mut F) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        finish_utf8(&mut self.byte_buffer, &mut self.text_buffer)?;
        process_final_stream_line(&mut self.text_buffer, &mut self.state, handle_delta)?;
        self.state.finalize()
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
mod tests;
