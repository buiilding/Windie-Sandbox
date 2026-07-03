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
        .load_messages(conversation_id)
        .context("failed to load conversation messages")?
        .last()
        .and_then(|message| message.id.clone());
    let model_messages =
        ContextBuilder::build(store, conversation_id).context("failed to build model context")?;

    output.start_assistant_message();
    let reply = llm
        .stream(&model_messages, |text| output.assistant_delta(text))
        .await
        .context("failed to stream assistant response")?;
    output.end_assistant_message();

    let assistant_message_id = store
        .save_message(
            conversation_id,
            parent_message_id.as_ref(),
            Role::Assistant,
            &reply,
            None,
        )
        .context("failed to save assistant message")?;

    Ok(Message {
        id: Some(assistant_message_id),
        parent_message_id,
        role: Role::Assistant,
        content: reply,
        metadata: None,
    })
}

#[allow(dead_code)]
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
