//! Model-facing context construction.
//!
//! This module decides what conversation history the LLM sees. Full history
//! stays in storage; compacted context can be built for model requests.

use anyhow::Result;

use crate::conversation::{ConversationId, Message, MessageView, Role};
use crate::store::Compaction;
use crate::store::Store;

const COMPACTION_PREFIX: &str = "Previous conversation summary:\n";

/// Builds the exact message list sent to the model.
pub struct ContextBuilder;

#[derive(Debug)]
/// Inputs needed to flatten model-facing context.
///
/// `perf.rs` can load these fields step by step and time each load without
/// putting benchmark timing logic inside this module.
pub struct ContextParts {
    pub active_path: Vec<Message>,
    pub system_prompt: Option<String>,
    pub compaction: Option<Compaction>,
}

impl ContextBuilder {
    /// Loads the active path unless a compaction checkpoint exists on that path.
    ///
    /// With a saved system prompt, the model sees that prompt first. With
    /// compaction, the model also sees one synthetic system summary plus the
    /// active-path messages after the checkpoint. The full uncompressed tree
    /// remains in SQLite.
    pub fn build(store: &Store, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let parts = Self::load_parts(store, conversation_id)?;

        Ok(Self::flatten(parts))
    }

    /// Builds context from a run's immutable prompt and compaction snapshot.
    pub fn build_with_configuration(
        store: &Store,
        conversation_id: &ConversationId,
        system_prompt: Option<String>,
        compaction: Option<Compaction>,
    ) -> Result<Vec<Message>> {
        Ok(Self::flatten(ContextParts {
            active_path: store.load_active_path(conversation_id)?,
            system_prompt,
            compaction,
        }))
    }

    /// Loads the storage-backed pieces needed to build model-facing context.
    ///
    /// This helper has no benchmark timers. Benchmark code can call each store
    /// method directly when it needs a lower-level timing breakdown.
    pub fn load_parts(store: &Store, conversation_id: &ConversationId) -> Result<ContextParts> {
        let active_path = store.load_active_path(conversation_id)?;
        let system_prompt = store.system_prompt(conversation_id)?;
        let compaction = store.latest_compaction(conversation_id)?;

        Ok(ContextParts {
            active_path,
            system_prompt,
            compaction,
        })
    }

    /// Flattens loaded context parts into the exact messages sent to the model.
    pub fn flatten(parts: ContextParts) -> Vec<Message> {
        let ContextParts {
            active_path,
            system_prompt,
            compaction,
        } = parts;

        flatten_context(
            active_path,
            system_prompt,
            compaction.as_ref(),
            |message| message.id.as_ref().map(|id| id.as_str()),
            system_prompt_message,
        )
    }

    /// Builds the metadata-only projection used by inspection without loading
    /// image bytes or duplicating model-context ordering rules.
    pub fn flatten_view(
        active_path: Vec<MessageView>,
        system_prompt: Option<String>,
        compaction: Option<&Compaction>,
    ) -> Vec<MessageView> {
        flatten_context(
            active_path,
            system_prompt,
            compaction,
            |message| message.id.as_deref(),
            system_prompt_message_view,
        )
    }
}

/// Applies compaction and system-prompt ordering to any message projection.
fn flatten_context<T>(
    active_path: Vec<T>,
    system_prompt: Option<String>,
    compaction: Option<&Compaction>,
    message_id: impl Fn(&T) -> Option<&str>,
    system_message: impl Fn(String) -> T,
) -> Vec<T> {
    let mut messages = if let Some(compaction) = compaction {
        if let Some(index) = active_path
            .iter()
            .position(|message| message_id(message) == Some(compaction.through_message_id.as_str()))
        {
            let mut compacted = vec![system_message(format!(
                "{COMPACTION_PREFIX}{}",
                compaction.content
            ))];
            compacted.extend(active_path.into_iter().skip(index + 1));
            compacted
        } else {
            active_path
        }
    } else {
        active_path
    };
    if let Some(system_prompt) = system_prompt {
        messages.insert(0, system_message(system_prompt));
    }
    messages
}

/// Converts a saved system prompt into a model-facing system message.
fn system_prompt_message(content: String) -> Message {
    Message {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content,
        parts: Vec::new(),
        metadata: None,
    }
}

/// Converts synthetic context text into a metadata-only system message.
fn system_prompt_message_view(content: String) -> MessageView {
    MessageView {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content,
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
        store
            .insert_message(
                &conversation_id,
                Some(&first_id),
                Role::Assistant,
                "hello back",
                None,
            )
            .unwrap();

        let messages = ContextBuilder::build(&store, &conversation_id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "hello back");
    }

    #[test]
    fn prepends_conversation_system_prompt() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        store
            .set_system_prompt(&conversation_id, "You are concise.")
            .unwrap();
        store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();

        let messages = ContextBuilder::build(&store, &conversation_id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[0].content, "You are concise.");
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
        store
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

    #[test]
    fn ignores_compaction_outside_active_path() {
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
        store
            .set_active_message(&conversation_id, &active_id)
            .unwrap();

        let messages = ContextBuilder::build(&store, &conversation_id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "root");
        assert_eq!(messages[1].content, "active");
    }
}
