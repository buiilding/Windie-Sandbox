use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::conversation::Message;

pub struct BifrostClient {
    http: Client,
    base_url: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

impl BifrostClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    pub fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub async fn stream<F>(&self, messages: &[Message], mut handle_delta: F) -> Result<String>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let request = ChatRequest {
            model: &self.model,
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
        let mut buffer = String::new();
        let mut answer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read chat stream")?;
            let text =
                std::str::from_utf8(&chunk).context("chat stream contained invalid utf-8")?;

            buffer.push_str(text);
            process_stream_lines(&mut buffer, &mut answer, &mut handle_delta)?;
        }

        process_final_stream_line(&mut buffer, &mut answer, &mut handle_delta)?;

        if answer.trim().is_empty() {
            return Err(anyhow!("chat stream did not include assistant content"));
        }

        Ok(answer)
    }
}

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
