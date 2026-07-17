//! Tree-wide system prompt persistence.
//!
//! One prompt per conversation, same for every branch/head.

use super::*;

impl Store {
    /// Loads the conversation-wide system prompt.
    pub fn system_prompt(&self, conversation_id: &ConversationId) -> Result<Option<String>> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "SELECT system_prompt FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .context("failed to load system prompt")
            .map(Option::flatten)
    }

    /// Sets or replaces the conversation-wide system prompt.
    ///
    /// Empty text clears the prompt (stores NULL).
    pub fn set_system_prompt(
        &mut self,
        conversation_id: &ConversationId,
        content: &str,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;

        let now = now_millis()?;
        let content = if content.trim().is_empty() {
            None
        } else {
            Some(content)
        };

        let transaction = self
            .connection
            .transaction()
            .context("failed to start system prompt transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET system_prompt = ?1 WHERE id = ?2",
                params![content, conversation_id.as_str()],
            )
            .context("failed to save system prompt")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit system prompt update")?;

        Ok(())
    }

    /// Clears the conversation-wide system prompt.
    pub fn remove_system_prompt(&mut self, conversation_id: &ConversationId) -> Result<()> {
        self.set_system_prompt(conversation_id, "")
    }
}
