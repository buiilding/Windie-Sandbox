//! Model-facing context construction.
//!
//! This module decides what conversation history the LLM sees. Full history
//! stays in storage; compacted context can be built for model requests.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::conversation::{ConversationId, Message, Role};
use crate::store::Store;

const COMPACTION_PREFIX: &str = "Previous conversation summary:\n";

/// Builds the exact message list sent to the model.
pub struct ContextBuilder;

#[derive(Debug)]
/// Model-facing context plus benchmark timing details.
pub struct ContextBuildProfile {
    pub messages: Vec<Message>,
    pub timings: ContextBuildTimings,
}

#[derive(Debug, Default)]
/// Timing details for model-facing context construction.
pub struct ContextBuildTimings {
    pub active_path_load: Duration,
    pub system_prompt_load: Duration,
    pub compaction_load: Duration,
    pub flatten: Duration,
}

impl ContextBuilder {
    /// Loads the active path unless a compaction checkpoint exists on that path.
    ///
    /// With a saved system prompt, the model sees that prompt first. With
    /// compaction, the model also sees one synthetic system summary plus the
    /// active-path messages after the checkpoint. The full uncompressed tree
    /// remains in SQLite.
    pub fn build(store: &Store, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        Ok(Self::build_profile(store, conversation_id)?.messages)
    }

    /// Builds model-facing context and records benchmark timing details.
    ///
    /// This uses the same behavior as `build`, but exposes where the time went:
    /// loading the active path, loading the conversation system prompt, checking
    /// for compaction, and flattening/prepending synthetic system messages.
    pub fn build_profile(
        store: &Store,
        conversation_id: &ConversationId,
    ) -> Result<ContextBuildProfile> {
        let active_path_started = Instant::now();
        let active_path = store.load_active_path(conversation_id)?;
        let active_path_load = active_path_started.elapsed();

        let system_prompt_started = Instant::now();
        let system_prompt = store.system_prompt(conversation_id)?;
        let system_prompt_load = system_prompt_started.elapsed();

        let compaction_started = Instant::now();
        let Some(compaction) = store.latest_compaction(conversation_id)? else {
            let flatten_started = Instant::now();
            let messages = with_system_prompt(system_prompt, active_path);
            return Ok(ContextBuildProfile {
                messages,
                timings: ContextBuildTimings {
                    active_path_load,
                    system_prompt_load,
                    compaction_load: compaction_started.elapsed(),
                    flatten: flatten_started.elapsed(),
                },
            });
        };
        let compaction_load = compaction_started.elapsed();

        let flatten_started = Instant::now();
        let Some(compaction_index) = active_path
            .iter()
            .position(|message| message.id.as_ref() == Some(&compaction.through_message_id))
        else {
            let messages = with_system_prompt(system_prompt, active_path);
            return Ok(ContextBuildProfile {
                messages,
                timings: ContextBuildTimings {
                    active_path_load,
                    system_prompt_load,
                    compaction_load,
                    flatten: flatten_started.elapsed(),
                },
            });
        };

        let mut messages = vec![compaction_message(&compaction.content)];
        messages.extend(active_path.into_iter().skip(compaction_index + 1));

        Ok(ContextBuildProfile {
            messages: with_system_prompt(system_prompt, messages),
            timings: ContextBuildTimings {
                active_path_load,
                system_prompt_load,
                compaction_load,
                flatten: flatten_started.elapsed(),
            },
        })
    }
}

/// Prepends the conversation-level system prompt when one is set.
fn with_system_prompt(system_prompt: Option<String>, messages: Vec<Message>) -> Vec<Message> {
    let Some(system_prompt) = system_prompt else {
        return messages;
    };

    let mut model_messages = Vec::with_capacity(messages.len() + 1);
    model_messages.push(system_prompt_message(system_prompt));
    model_messages.extend(messages);

    model_messages
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
        let conversation_id = store.create_conversation().unwrap();

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
        let conversation_id = store.create_conversation().unwrap();
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
        let conversation_id = store.create_conversation().unwrap();

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
        let conversation_id = store.create_conversation().unwrap();

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
