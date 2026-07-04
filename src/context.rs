//! Model-facing context construction.
//!
//! This module decides what conversation history the LLM sees. Full history
//! stays in storage; compacted context can be built for model requests.

use anyhow::Result;

use crate::conversation::{ConversationId, Message, Role};
use crate::store::Store;

const COMPACTION_PREFIX: &str = "Previous conversation summary:\n";

/// Builds the exact message list sent to the model.
pub struct ContextBuilder;

impl ContextBuilder {
    /// Loads full history unless a compaction checkpoint exists.
    ///
    /// With compaction, the model sees one synthetic system summary plus the
    /// messages after the checkpoint. The full uncompressed history remains in
    /// SQLite.
    pub fn build(store: &Store, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let Some(compaction) = store.latest_compaction(conversation_id)? else {
            return store.load_messages(conversation_id);
        };

        let mut messages = vec![compaction_message(&compaction.content)];
        messages.extend(
            store.load_messages_after(conversation_id, Some(&compaction.through_message_id))?,
        );

        Ok(messages)
    }
}

/// Converts a saved compaction into a system message for the model.
fn compaction_message(content: &str) -> Message {
    Message {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content: format!("{COMPACTION_PREFIX}{content}"),
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_full_context_without_compaction() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation().unwrap();

        store
            .append_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();
        store
            .append_message(&conversation_id, None, Role::Assistant, "hello back", None)
            .unwrap();

        let messages = ContextBuilder::build(&store, &conversation_id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "hello back");
    }

    #[test]
    fn builds_compacted_context_from_latest_checkpoint() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation().unwrap();

        let first_id = store
            .append_message(&conversation_id, None, Role::User, "one", None)
            .unwrap();
        let second_id = store
            .append_message(
                &conversation_id,
                Some(&first_id),
                Role::Assistant,
                "two",
                None,
            )
            .unwrap();
        store
            .append_message(
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

        let messages = ContextBuilder::build(&store, &conversation_id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(
            messages[0].content,
            "Previous conversation summary:\none and two happened"
        );
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[1].content, "three");
    }
}
