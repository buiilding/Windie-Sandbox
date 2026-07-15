//! Model-facing context construction.
//!
//! This module decides what conversation history the LLM sees. Full history
//! stays in storage; compacted context can be built for model requests.

use anyhow::Result;

use crate::conversation::{ConversationId, Message, MessageId, Role};
use crate::store::Compaction;
use crate::store::Store;
use crate::tool::ToolSchema;

const COMPACTION_PREFIX: &str = "Previous conversation summary:\n";

/// Builds the exact message list sent to the model.
pub struct ContextBuilder;

#[derive(Debug)]
/// Complete model-facing context for one selected conversation path.
///
/// Runtime callers use this shape so messages and tool schemas are resolved
/// from the same path head. That keeps branch-local system messages and tools
/// from leaking across sibling branches.
pub struct ModelContext {
    pub messages: Vec<Message>,
    pub tool_schemas: Vec<ToolSchema>,
}

#[derive(Debug)]
/// Inputs needed to flatten model-facing context.
///
/// `perf.rs` can load these fields step by step and time each load without
/// putting benchmark timing logic inside this module.
pub struct ContextParts {
    pub path: Vec<Message>,
    pub compaction: Option<Compaction>,
}

impl ContextBuilder {
    /// Loads the model-facing messages for an explicit path head.
    ///
    /// System prompt edits are normal `Role::System` messages in the selected
    /// path. The model sees only the latest non-empty system message first.
    /// With compaction, the model also sees one synthetic system summary plus
    /// path messages after the checkpoint. The full uncompressed tree remains
    /// in SQLite.
    pub fn build_messages(
        store: &Store,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<Vec<Message>> {
        let mut path = store.load_root_system_messages(conversation_id)?;
        if let Some(message_id) = head_message_id {
            path.extend(store.load_path_to_message(conversation_id, message_id)?);
        }
        let compaction = store.latest_compaction(conversation_id)?;

        Ok(Self::flatten(ContextParts { path, compaction }))
    }

    /// Loads the model context for an explicit path head.
    ///
    /// This is the runtime entrypoint. It resolves messages and tool schemas
    /// against the same captured head.
    pub fn build_model_context(
        store: &Store,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<ModelContext> {
        Ok(ModelContext {
            messages: Self::build_messages(store, conversation_id, head_message_id)?,
            tool_schemas: store.load_tool_schemas_for_head(conversation_id, head_message_id)?,
        })
    }

    /// Flattens loaded context parts into the exact messages sent to the model.
    pub fn flatten(parts: ContextParts) -> Vec<Message> {
        let ContextParts { path, compaction } = parts;

        let Some(compaction) = compaction else {
            return model_messages_from_path(path, None);
        };
        let effective_system_message = effective_system_message(&path);
        let Some(compaction_index) = path
            .iter()
            .position(|message| message.id.as_ref() == Some(&compaction.through_message_id))
        else {
            return model_messages_from_path(path, None);
        };

        model_messages_from_path(
            path.into_iter().skip(compaction_index + 1).collect(),
            Some((
                effective_system_message,
                compaction_message(&compaction.content),
            )),
        )
    }
}

/// Builds provider-facing messages from one path segment.
///
/// System prompt edits are persisted as normal tree messages. Providers should
/// still receive a single current system instruction, so historical system
/// messages are removed and the latest non-empty one is placed first. An empty
/// latest system message acts as a branch-local clear marker.
fn model_messages_from_path(
    messages: Vec<Message>,
    compaction: Option<(Option<Message>, Message)>,
) -> Vec<Message> {
    let effective_system_message = compaction
        .as_ref()
        .map(|(message, _)| message.clone())
        .unwrap_or_else(|| effective_system_message(&messages));
    let mut model_messages = Vec::with_capacity(messages.len() + usize::from(compaction.is_some()));

    if let Some(system_message) = effective_system_message {
        model_messages.push(system_message);
    }
    if let Some((_, compaction_message)) = compaction {
        model_messages.push(compaction_message);
    }

    model_messages.extend(
        messages
            .into_iter()
            .filter(|message| message.role != Role::System),
    );

    model_messages
}

/// Returns the effective prompt message from a path.
fn effective_system_message(messages: &[Message]) -> Option<Message> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == Role::System)
        .filter(|message| !message.content.trim().is_empty())
        .cloned()
}

/// Converts a saved compaction into a system message for the model.
fn compaction_message(content: &str) -> Message {
    Message {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content: format!("{COMPACTION_PREFIX}{content}"),
        parts: Vec::new(),
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_full_context_without_compaction() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();

        let first_id = store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();
        let second_id = store
            .insert_message(
                &conversation_id,
                Some(&first_id),
                Role::Assistant,
                "hello back",
                None,
            )
            .unwrap();

        let messages =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&second_id)).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "hello back");
    }

    #[test]
    fn prepends_latest_system_message() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let prompt_id = store
            .set_system_prompt(&conversation_id, "You are concise.")
            .unwrap();
        let user_id = store
            .insert_message(
                &conversation_id,
                Some(&prompt_id),
                Role::User,
                "hello",
                None,
            )
            .unwrap();

        let messages =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&user_id)).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[0].content, "You are concise.");
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "hello");
    }

    #[test]
    fn only_latest_system_message_reaches_model() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let first_id = store
            .set_system_prompt(&conversation_id, "old prompt")
            .unwrap();
        let user_id = store
            .insert_message(&conversation_id, Some(&first_id), Role::User, "hello", None)
            .unwrap();
        let prompt_id = store
            .set_system_prompt_at_head(&conversation_id, Some(&user_id), "new prompt")
            .unwrap();

        let messages =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&prompt_id)).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[0].content, "new prompt");
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "hello");
    }

    #[test]
    fn builds_compacted_context_from_latest_checkpoint() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();

        let first_id = store
            .insert_message(&conversation_id, None, Role::User, "one", None)
            .unwrap();
        let second_id = store
            .insert_message(
                &conversation_id,
                Some(&first_id),
                Role::Assistant,
                "two",
                None,
            )
            .unwrap();
        let third_id = store
            .insert_message(
                &conversation_id,
                Some(&second_id),
                Role::User,
                "three",
                None,
            )
            .unwrap();
        store
            .save_compaction(&conversation_id, &second_id, "one and two happened")
            .unwrap();

        let messages =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&third_id)).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(
            messages[0].content,
            "Previous conversation summary:\none and two happened"
        );
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "three");
    }

    #[test]
    fn ignores_compaction_outside_requested_path() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();

        let root_id = store
            .insert_message(&conversation_id, None, Role::User, "root", None)
            .unwrap();
        let inactive_id = store
            .insert_message(
                &conversation_id,
                Some(&root_id),
                Role::Assistant,
                "inactive",
                None,
            )
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
            .save_compaction(&conversation_id, &inactive_id, "inactive summary")
            .unwrap();
        let messages =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&active_id)).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "root");
        assert_eq!(messages[1].content, "active");
    }
}
