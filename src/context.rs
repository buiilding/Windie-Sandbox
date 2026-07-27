//! Model-facing context construction.
//!
//! This module decides what conversation history the LLM sees. Full history
//! stays in storage; compaction can be built for model requests.
//! System prompt and tool schemas are tree-wide (conversation-wide), same for any head.

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
/// Messages are head-dependent (path to head). Tool schemas and system prompt
/// are tree-wide (conversation-wide) and same for any head.
pub struct ModelContext {
    pub messages: Vec<Message>,
    pub tool_schemas: Vec<ToolSchema>,
}

#[derive(Debug)]
/// Inputs needed to flatten model-facing context.
pub struct ContextParts {
    pub path: Vec<Message>,
    pub system_prompt: Option<String>,
    pub compaction: Option<Compaction>,
}

impl ContextBuilder {
    /// Loads the model-facing messages for an explicit path head.
    ///
    /// Tree-wide: system prompt comes from conversations.system_prompt column,
    /// not from branch-local system messages.
    pub fn build_messages(
        store: &Store,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<Vec<Message>> {
        let path = match head_message_id {
            Some(message_id) => store.load_path_to_message(conversation_id, message_id)?,
            None => Vec::new(),
        };
        let system_prompt = store.system_prompt(conversation_id)?;
        let compaction = store.latest_compaction(conversation_id)?;

        Ok(Self::flatten(ContextParts {
            path,
            system_prompt,
            compaction,
        }))
    }

    /// Loads the full model context for an explicit path head.
    ///
    /// Tree-wide: tools are conversation-wide, messages are head-dependent.
    pub fn build_model_context(
        store: &Store,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<ModelContext> {
        Ok(ModelContext {
            messages: Self::build_messages(store, conversation_id, head_message_id)?,
            tool_schemas: store.load_tool_schemas(conversation_id)?,
        })
    }

    /// Flattens loaded context parts into messages sent to model.
    pub fn flatten(parts: ContextParts) -> Vec<Message> {
        let ContextParts {
            path,
            system_prompt,
            compaction,
        } = parts;

        let Some(compaction) = compaction else {
            return with_system_prompt(system_prompt, path);
        };
        let Some(compaction_index) = path
            .iter()
            .position(|message| message.id.as_ref() == Some(&compaction.through_message_id))
        else {
            return with_system_prompt(system_prompt, path);
        };

        let mut messages = vec![compaction_message(&compaction.content)];
        messages.extend(path.into_iter().skip(compaction_index + 1));

        with_system_prompt(system_prompt, messages)
    }
}

/// Prepends the conversation-wide system prompt when one is set.
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
    fn prepends_system_message() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        store
            .set_system_prompt(&conversation_id, "You are concise.")
            .unwrap();
        let user_id = store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();

        let messages =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&user_id)).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[0].content, "You are concise.");
        assert_eq!(messages[1].role, Role::User);
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

    #[test]
    fn system_prompt_is_tree_wide_same_for_any_head() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let root_id = store
            .insert_message(&conversation_id, None, Role::User, "root", None)
            .unwrap();
        let branch_a = store
            .insert_message(&conversation_id, Some(&root_id), Role::User, "a", None)
            .unwrap();
        let branch_b = store
            .insert_message(&conversation_id, Some(&root_id), Role::User, "b", None)
            .unwrap();
        store
            .set_system_prompt(&conversation_id, "global prompt")
            .unwrap();

        let messages_a =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&branch_a)).unwrap();
        let messages_b =
            ContextBuilder::build_messages(&store, &conversation_id, Some(&branch_b)).unwrap();

        assert_eq!(messages_a[0].role, Role::System);
        assert_eq!(messages_a[0].content, "global prompt");
        assert_eq!(messages_b[0].role, Role::System);
        assert_eq!(messages_b[0].content, "global prompt");
    }
}
