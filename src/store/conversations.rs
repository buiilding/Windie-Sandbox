//! Conversations persistence owned by the store module.

use super::*;

impl Store {
    /// Creates an empty conversation with a generated ID and persisted model.
    pub fn create_conversation(&self, model: &str) -> Result<ConversationId> {
        let model = normalize_conversation_model(model)?;
        let id = ConversationId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT INTO conversations (
                    id,
                    model,
                    reasoning_effort,
                    active_message_id,
                    system_prompt,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, NULL, NULL, NULL, ?3, ?4, ?4)
                ",
                params![
                    id.as_str(),
                    model,
                    ToolApprovalMode::Manual.as_storage(),
                    now
                ],
            )
            .context("failed to create conversation")?;

        Ok(id)
    }

    #[cfg(test)]
    /// Creates a deterministic conversation ID for tests that need predictable
    /// setup.
    pub(crate) fn get_or_create_default_conversation(&self, model: &str) -> Result<ConversationId> {
        let model = normalize_conversation_model(model)?;
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT OR IGNORE INTO conversations (
                    id,
                    model,
                    reasoning_effort,
                    active_message_id,
                    system_prompt,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, NULL, NULL, NULL, ?3, ?4, ?4)
                ",
                params![
                    DEFAULT_CONVERSATION_ID,
                    model,
                    ToolApprovalMode::Manual.as_storage(),
                    now
                ],
            )
            .context("failed to create default conversation")?;

        Ok(ConversationId::new(DEFAULT_CONVERSATION_ID))
    }

    /// Lists conversations with message counts without loading every message.
    pub fn list_conversations(&self) -> Result<Vec<ConversationInfo>> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT
                    conversations.id,
                    conversations.model,
                    COUNT(messages.id) AS message_count
                FROM conversations
                LEFT JOIN messages ON messages.conversation_id = conversations.id
                GROUP BY conversations.id
                ORDER BY conversations.updated_at DESC, conversations.rowid DESC
                ",
            )
            .context("failed to prepare conversation list")?;

        let conversations = statement
            .query_map([], |row| {
                Ok(ConversationInfo {
                    id: ConversationId::new(row.get::<_, String>(0)?),
                    model: row.get(1)?,
                    message_count: row.get(2)?,
                })
            })
            .context("failed to list conversations")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read conversations")?;

        Ok(conversations)
    }

    /// Loads all messages for one conversation in stable insertion order.
    pub fn load_messages(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let mut messages = self.load_message_rows(conversation_id)?;
        self.attach_message_parts(&mut messages)
            .context("failed to load message parts")?;

        Ok(messages)
    }

    /// Loads all stored messages for tree inspection.
    pub fn load_message_tree(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        self.load_messages(conversation_id)
    }

    /// Loads the active message ID for one conversation.
    pub fn active_message_id(&self, conversation_id: &ConversationId) -> Result<Option<MessageId>> {
        self.ensure_conversation_exists(conversation_id)?;

        let active_message_id = self
            .connection
            .query_row(
                "SELECT active_message_id FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .context("failed to load active message")?
            .flatten()
            .map(MessageId::new);

        Ok(active_message_id)
    }

    /// Loads the conversation-level system prompt.
    ///
    /// The system prompt is not part of the message tree. Context construction
    /// prepends it to the model-facing messages when it exists.
    pub fn system_prompt(&self, conversation_id: &ConversationId) -> Result<Option<String>> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "SELECT system_prompt FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load system prompt")
            .map(Option::flatten)
    }

    /// Loads the conversation's persisted default model.
    pub fn conversation_model(&self, conversation_id: &ConversationId) -> Result<String> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "SELECT model FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get(0),
            )
            .context("failed to load conversation model")
    }

    /// Loads the conversation-level reasoning effort for future queries.
    ///
    /// The store persists only the user/client-selected effort string. Provider
    /// request shaping, such as adding OpenAI's visible reasoning-summary flag,
    /// stays in the operation/LLM boundary where the concrete model is known.
    pub fn conversation_reasoning_effort(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<String>> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "SELECT reasoning_effort FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load conversation reasoning effort")
            .map(Option::flatten)
    }

    /// Sets the conversation's persisted default model.
    pub fn set_conversation_model(
        &mut self,
        conversation_id: &ConversationId,
        model: &str,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let model = normalize_conversation_model(model)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation model transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET model = ?1, reasoning_effort = NULL WHERE id = ?2",
                params![model, conversation_id.as_str()],
            )
            .context("failed to save conversation model")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit conversation model update")?;

        Ok(())
    }

    /// Sets the conversation-level reasoning effort used by future queries.
    ///
    /// `None` and blank strings clear the setting. The store intentionally does
    /// not validate model-specific values because Bifrost model metadata is the
    /// source of truth for which efforts are available for a selected model.
    pub fn set_conversation_reasoning_effort(
        &mut self,
        conversation_id: &ConversationId,
        effort: Option<&str>,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let effort = normalize_conversation_reasoning_effort(effort);

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation reasoning transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET reasoning_effort = ?1 WHERE id = ?2",
                params![effort, conversation_id.as_str()],
            )
            .context("failed to save conversation reasoning effort")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit conversation reasoning update")?;

        Ok(())
    }

    /// Loads the conversation default for tool-call approval.
    pub fn tool_approval_mode(&self, conversation_id: &ConversationId) -> Result<ToolApprovalMode> {
        self.ensure_conversation_exists(conversation_id)?;

        let value = self
            .connection
            .query_row(
                "SELECT tool_approval_mode FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .context("failed to load tool approval mode")?;

        ToolApprovalMode::from_storage(&value)
            .ok_or_else(|| anyhow!("unknown tool approval mode: {value}"))
    }

    /// Sets the conversation default for future tool-call approvals.
    pub fn set_tool_approval_mode(
        &mut self,
        conversation_id: &ConversationId,
        mode: ToolApprovalMode,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start tool approval mode transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET tool_approval_mode = ?1 WHERE id = ?2",
                params![mode.as_storage(), conversation_id.as_str()],
            )
            .context("failed to save tool approval mode")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit tool approval mode update")?;

        Ok(())
    }

    /// Sets or replaces the conversation-level system prompt.
    ///
    /// Empty text clears the prompt by storing `NULL`. This keeps `set
    /// systemprompt` idempotent: callers can set it before any messages exist
    /// or replace it after a prompt already exists.
    pub fn set_system_prompt(
        &mut self,
        conversation_id: &ConversationId,
        content: &str,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;

        let now = now_millis()?;
        let content = if content.is_empty() {
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

    /// Clears the conversation-level system prompt without changing messages.
    pub fn remove_system_prompt(&mut self, conversation_id: &ConversationId) -> Result<()> {
        self.set_system_prompt(conversation_id, "")
    }
    /// Deletes one conversation and all messages/compactions owned by it.
    pub fn remove_conversation(&mut self, conversation_id: &ConversationId) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;

        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation delete transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET active_message_id = NULL WHERE id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to clear active message")?;
        transaction
            .execute(
                "DELETE FROM compactions WHERE conversation_id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation compactions")?;
        transaction
            .execute(
                "DELETE FROM tool_schemas WHERE conversation_id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation tool schemas")?;
        transaction
            .execute(
                "DELETE FROM messages WHERE conversation_id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation messages")?;
        delete_orphan_image_assets_in_transaction(&transaction)
            .context("failed to delete orphan image assets")?;
        transaction
            .execute(
                "DELETE FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation")?;
        transaction
            .commit()
            .context("failed to commit conversation delete")?;

        Ok(())
    }
}
