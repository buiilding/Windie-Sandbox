//! Runtime flow coordination.
//!
//! Coordinates runtime flows across output, store, context, and LLM components.
//! One-shot query primitives live here so CLI and future UI clients can reuse
//! the same execution path.

use anyhow::{Context, Result};

use crate::context::ContextBuilder;
use crate::conversation::{ConversationId, Message, Role};
use crate::llm::RuntimeLlm;
use crate::output::RuntimeOutput;
use crate::store::Store;

/// Runs one query against one existing conversation.
///
/// Data flow:
/// load active message -> build model context from active path -> stream LLM
/// output -> save the assistant reply. The assistant message is only persisted
/// after successful inference and becomes the new active message.
pub(crate) async fn query_conversation<O, L>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    let parent_message_id = store
        .active_message_id(conversation_id)
        .context("failed to load active message")?;
    let model_messages =
        ContextBuilder::build(store, conversation_id).context("failed to build model context")?;
    let tool_schemas = store
        .load_tool_schemas(conversation_id)
        .context("failed to load tool schemas")?;

    output.start_assistant_message();
    let assistant_response = llm
        .stream(&model_messages, &tool_schemas, |text| {
            output.assistant_delta(text)
        })
        .await
        .context("failed to stream assistant response")?;
    output.end_assistant_message();
    output.assistant_tool_calls(&assistant_response.metadata.tool_calls);

    let metadata = if assistant_response.metadata.is_empty() {
        None
    } else {
        Some(assistant_response.metadata)
    };
    let assistant_message_id = store
        .insert_message(
            conversation_id,
            parent_message_id.as_ref(),
            Role::Assistant,
            &assistant_response.content,
            metadata.as_ref(),
        )
        .context("failed to save assistant message")?;

    Ok(Message {
        id: Some(assistant_message_id),
        parent_message_id,
        role: Role::Assistant,
        content: assistant_response.content,
        parts: Vec::new(),
        metadata,
    })
}

#[allow(dead_code)]
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
