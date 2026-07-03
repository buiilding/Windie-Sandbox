//! Tests for runtime flow coordination.

use anyhow::{Error, Result, anyhow};

use super::*;
use crate::conversation::Message;

struct NoopOutput;

impl RuntimeOutput for NoopOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    fn end_assistant_message(&self) {}
}

struct FailingLlm;

impl RuntimeLlm for FailingLlm {
    async fn stream<F>(&self, _messages: &[Message], _handle_delta: F) -> Result<String>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Err(anyhow!("llm failed"))
    }
}

struct ReplyLlm {
    reply: String,
}

impl ReplyLlm {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
        }
    }
}

impl RuntimeLlm for ReplyLlm {
    async fn stream<F>(&self, _messages: &[Message], mut handle_delta: F) -> Result<String>
    where
        F: FnMut(&str) -> Result<()>,
    {
        handle_delta(&self.reply)?;

        Ok(self.reply.clone())
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
        .save_message(&conversation_id, None, Role::User, "hello", None)
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
