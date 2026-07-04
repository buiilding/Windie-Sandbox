//! OpenAI-compatible streaming LLM client.
//!
//! This module owns HTTP requests to Bifrost's chat completions endpoint and
//! parsing streamed response chunks.

use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::conversation::Message;

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
    async fn stream<F>(&self, messages: &[Message], handle_delta: F) -> Result<String>
    where
        F: FnMut(&str) -> Result<()>;
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
    messages: &'a [Message],
    stream: bool,
}

#[derive(Debug, Deserialize)]
/// One streamed server-sent event payload from the provider adapter.
struct ChatStreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
/// One candidate choice inside a streamed chat chunk.
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
/// Incremental assistant content inside a streamed choice.
struct StreamDelta {
    content: Option<String>,
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

    /// Sends the chat request, streams assistant deltas to the caller, and
    /// returns the full assistant response.
    pub async fn stream<F>(&self, messages: &[Message], mut handle_delta: F) -> Result<String>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let request = ChatRequest {
            model: self.model.as_str(),
            messages,
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
        let mut answer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read chat stream")?;

            // Network chunks can split inside UTF-8 characters or SSE lines, so
            // bytes are decoded separately from line parsing.
            byte_buffer.extend_from_slice(&chunk);
            append_valid_utf8(&mut byte_buffer, &mut buffer)?;
            process_stream_lines(&mut buffer, &mut answer, &mut handle_delta)?;
        }

        finish_utf8(&mut byte_buffer, &mut buffer)?;
        process_final_stream_line(&mut buffer, &mut answer, &mut handle_delta)?;

        if answer.trim().is_empty() {
            return Err(anyhow!("chat stream did not include assistant content"));
        }

        Ok(answer)
    }
}

impl RuntimeLlm for BifrostClient {
    async fn stream<F>(&self, messages: &[Message], handle_delta: F) -> Result<String>
    where
        F: FnMut(&str) -> Result<()>,
    {
        BifrostClient::stream(self, messages, handle_delta).await
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
    answer: &mut String,
    handle_delta: &mut F,
) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    while let Some(line_end) = buffer.find('\n') {
        let line = buffer[..line_end].trim_end_matches('\r').to_string();
        buffer.drain(..=line_end);
        process_stream_line(&line, answer, handle_delta)?;
    }

    Ok(())
}

/// Processes one final stream line when the server closes without a trailing
/// newline.
fn process_final_stream_line<F>(
    buffer: &mut String,
    answer: &mut String,
    handle_delta: &mut F,
) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    if buffer.trim().is_empty() {
        return Ok(());
    }

    let line = std::mem::take(buffer);
    process_stream_line(line.trim_end_matches('\r'), answer, handle_delta)
}

/// Parses one SSE line and forwards assistant content deltas.
fn process_stream_line<F>(line: &str, answer: &mut String, handle_delta: &mut F) -> Result<()>
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
        if let Some(content) = choice.delta.content {
            handle_delta(&content)?;
            answer.push_str(&content);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_removes_trailing_slash() {
        let base_url = BaseUrl::new("http://localhost:8080/v1/");

        assert_eq!(base_url.as_str(), "http://localhost:8080/v1");
    }

    #[test]
    fn model_name_preserves_provider_prefix() {
        let model = ModelName::new("openai/gpt-4o-mini");

        assert_eq!(model.as_str(), "openai/gpt-4o-mini");
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
    fn parses_stream_content_delta() {
        let mut answer = String::new();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_line(
            r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
            &mut answer,
            &mut handle_delta,
        )
        .unwrap();

        assert_eq!(answer, "Hello");
        assert_eq!(deltas, vec!["Hello"]);
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
        let mut answer = String::new();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_line("data: [DONE]", &mut answer, &mut handle_delta).unwrap();

        assert!(answer.is_empty());
        assert!(deltas.is_empty());
    }

    #[test]
    fn ignores_non_data_stream_line() {
        let mut answer = String::new();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_line("event: message", &mut answer, &mut handle_delta).unwrap();

        assert!(answer.is_empty());
        assert!(deltas.is_empty());
    }

    #[test]
    fn accumulates_multiple_stream_lines() {
        let mut buffer = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\
             data: [DONE]\n"
            .to_string();
        let mut answer = String::new();
        let mut deltas = Vec::new();
        let mut handle_delta = |text: &str| {
            deltas.push(text.to_string());
            Ok(())
        };

        process_stream_lines(&mut buffer, &mut answer, &mut handle_delta).unwrap();

        assert!(buffer.is_empty());
        assert_eq!(answer, "Hello");
        assert_eq!(deltas, vec!["Hel", "lo"]);
    }

    #[test]
    fn keeps_partial_line_in_buffer() {
        let mut buffer = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}".to_string();
        let mut answer = String::new();
        let mut deltas = Vec::new();

        {
            let mut handle_delta = |text: &str| {
                deltas.push(text.to_string());
                Ok(())
            };

            process_stream_lines(&mut buffer, &mut answer, &mut handle_delta).unwrap();
        }

        assert!(!buffer.is_empty());
        assert!(answer.is_empty());
        assert!(deltas.is_empty());

        {
            let mut handle_delta = |text: &str| {
                deltas.push(text.to_string());
                Ok(())
            };

            process_final_stream_line(&mut buffer, &mut answer, &mut handle_delta).unwrap();
        }

        assert!(buffer.is_empty());
        assert_eq!(answer, "Hello");
        assert_eq!(deltas, vec!["Hello"]);
    }
}
