//! Tests for runtime flow coordination.

use anyhow::{Error, Result, anyhow};
use std::sync::Mutex;

use super::*;
use crate::conversation::{Message, MessageMetadata, ToolCall, ToolSchema, ToolSchemaName};
use crate::llm::{AssistantResponse, FinishReason};

struct NoopOutput;

impl RuntimeOutput for NoopOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[ToolCall]) {}
}

struct FailingLlm;

impl RuntimeLlm for FailingLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Err(anyhow!("llm failed"))
    }
}

struct ReplyLlm {
    reply: String,
}

struct CapturingLlm {
    messages: Mutex<Vec<Message>>,
    tools: Mutex<Vec<ToolSchema>>,
}

impl CapturingLlm {
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            tools: Mutex::new(Vec::new()),
        }
    }
}

impl RuntimeLlm for CapturingLlm {
    async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        *self.messages.lock().unwrap() = messages.to_vec();
        *self.tools.lock().unwrap() = tools.to_vec();
        handle_delta("captured")?;

        Ok(AssistantResponse {
            content: "captured".to_string(),
            metadata: MessageMetadata::default(),
            finish_reason: Some(FinishReason::Stop),
        })
    }
}

impl ReplyLlm {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
        }
    }
}

impl RuntimeLlm for ReplyLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        handle_delta(&self.reply)?;

        Ok(AssistantResponse {
            content: self.reply.clone(),
            metadata: MessageMetadata::default(),
            finish_reason: Some(FinishReason::Stop),
        })
    }
}

struct ToolCallLlm;

impl RuntimeLlm for ToolCallLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Ok(AssistantResponse {
            content: String::new(),
            metadata: MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"ls"}"#,
                )],
                ..Default::default()
            },
            finish_reason: Some(FinishReason::ToolCalls),
        })
    }
}

fn assert_error_chain(error: &Error, message: &str, cause: &str) {
    assert_eq!(error.to_string(), message);
    assert!(error.chain().any(|item| item.to_string() == cause));
}

#[tokio::test]
async fn query_conversation_saves_assistant_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let assistant_message = query_conversation(
        &NoopOutput,
        &ReplyLlm::new("hello back"),
        &mut store,
        &conversation_id,
    )
    .await
    .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(assistant_message.role, Role::Assistant);
    assert_eq!(assistant_message.content, "hello back");
    assert_eq!(
        assistant_message.parent_message_id.as_deref(),
        Some(user_id.as_str())
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].content, "hello back");
    assert_eq!(
        messages[1].parent_message_id.as_deref(),
        messages[0].id.as_deref()
    );
    assert_eq!(
        store
            .active_message_id(&conversation_id)
            .unwrap()
            .as_deref(),
        messages[1].id.as_deref()
    );
}

#[tokio::test]
async fn query_conversation_uses_active_path() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let active_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "active",
            None,
        )
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "inactive",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &active_id)
        .unwrap();
    let llm = CapturingLlm::new();

    query_conversation(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();

    let captured = llm.messages.lock().unwrap();

    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].content, "root");
    assert_eq!(captured[1].content, "active");
}

#[tokio::test]
async fn query_conversation_passes_tool_schemas_to_llm() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };
    store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    let llm = CapturingLlm::new();

    query_conversation(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();

    assert_eq!(*llm.tools.lock().unwrap(), vec![tool_schema]);
}

#[tokio::test]
async fn query_conversation_saves_tool_calls_in_metadata() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();

    let assistant_message =
        query_conversation(&NoopOutput, &ToolCallLlm, &mut store, &conversation_id)
            .await
            .unwrap();
    let metadata = assistant_message.metadata.as_ref().unwrap();

    assert!(assistant_message.content.is_empty());
    assert_eq!(metadata.tool_calls.len(), 1);
    assert_eq!(metadata.tool_calls[0].id.as_str(), "call_123");
    assert_eq!(metadata.tool_calls[0].name(), "run_shell");
    assert_eq!(metadata.tool_calls[0].arguments(), r#"{"command":"ls"}"#);
}

#[tokio::test]
async fn query_conversation_reports_llm_failure() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    let error = query_conversation(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
        .await
        .unwrap_err();

    assert_error_chain(&error, "failed to stream assistant response", "llm failed");
}
