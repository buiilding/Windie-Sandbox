//! Branch-scoped system prompt persistence.

use super::*;

impl Store {
    /// Loads root-scoped system prompt messages for default context resolution.
    pub fn load_root_system_messages(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<Message>> {
        self.ensure_conversation_exists(conversation_id)?;

        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_message_id, role, content, metadata
                FROM messages
                WHERE conversation_id = ?1
                  AND parent_message_id IS NULL
                  AND role = 'system'
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare root system message load")?;

        let mut messages = statement
            .query_map(params![conversation_id.as_str()], read_message_row)
            .context("failed to load root system messages")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read root system messages")?;
        self.attach_message_parts(&mut messages)
            .context("failed to load root system message parts")?;

        Ok(messages)
    }

    /// Loads the root-scoped effective system prompt.
    ///
    /// System prompts are stored as normal `Role::System` messages. Call
    /// `effective_system_prompt_for_head` to resolve a branch-local prompt for
    /// a specific message path.
    pub fn system_prompt(&self, conversation_id: &ConversationId) -> Result<Option<String>> {
        self.effective_system_prompt_for_head(conversation_id, None)
    }

    /// Loads the effective system prompt for an explicit conversation path.
    pub fn effective_system_prompt_for_head(
        &self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<Option<String>> {
        let mut messages = self.load_root_system_messages(conversation_id)?;
        if let Some(message_id) = head_message_id {
            messages.extend(self.load_path_to_message(conversation_id, message_id)?);
        }
        let prompt = messages
            .iter()
            .rev()
            .find(|message| message.role == Role::System)
            .and_then(|message| {
                let text = message.content.trim();
                (!text.is_empty()).then(|| message.content.clone())
            });

        Ok(prompt)
    }

    /// Inserts a root-scoped system message.
    pub fn set_system_prompt(
        &mut self,
        conversation_id: &ConversationId,
        content: &str,
    ) -> Result<MessageId> {
        self.set_system_prompt_at_head(conversation_id, None, content)
    }

    /// Inserts a system message at an explicit path head.
    ///
    /// Empty content is a normal persisted system message that means “clear the
    /// effective system prompt for this branch.”
    pub fn set_system_prompt_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        content: &str,
    ) -> Result<MessageId> {
        self.insert_message(
            conversation_id,
            parent_message_id,
            Role::System,
            content,
            None,
        )
    }

    /// Clears the root-scoped system prompt.
    pub fn remove_system_prompt(&mut self, conversation_id: &ConversationId) -> Result<MessageId> {
        self.remove_system_prompt_at_head(conversation_id, None)
    }

    /// Clears the system prompt at an explicit path head.
    pub fn remove_system_prompt_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
    ) -> Result<MessageId> {
        self.set_system_prompt_at_head(conversation_id, parent_message_id, "")
    }
}
