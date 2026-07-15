//! Conversation row persistence and conversation-level settings.

use super::*;

#[cfg(test)]
const DEFAULT_CONVERSATION_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
/// Lightweight row used by conversation listing.
pub struct ConversationInfo {
    pub id: ConversationId,
    pub title: Option<String>,
    pub model: String,
    pub message_count: i64,
}

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
                    title,
                    model,
                    reasoning_effort,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, NULL, ?2, NULL, ?3, ?4, ?4)
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
                    title,
                    model,
                    reasoning_effort,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, NULL, ?2, NULL, ?3, ?4, ?4)
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
                    conversations.title,
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
                    title: row.get(1)?,
                    model: row.get(2)?,
                    message_count: row.get(3)?,
                })
            })
            .context("failed to list conversations")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read conversations")?;

        Ok(conversations)
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

    /// Deletes one conversation and all messages/compactions owned by it.
    pub fn remove_conversation(&mut self, conversation_id: &ConversationId) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;

        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation delete transaction")?;

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

    /// Returns an error instead of silently treating missing conversations as
    /// empty.
    pub(super) fn ensure_conversation_exists(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<()> {
        if !self.conversation_exists(conversation_id)? {
            return Err(error::not_found(format!(
                "conversation does not exist: {conversation_id}"
            )));
        }

        Ok(())
    }

    /// Checks whether one conversation row exists.
    pub(super) fn conversation_exists(&self, conversation_id: &ConversationId) -> Result<bool> {
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |_| Ok(()),
            )
            .optional()
            .context("failed to check conversation")?
            .is_some();

        Ok(exists)
    }
}

/// Normalizes and validates the persisted model name for a conversation.
fn normalize_conversation_model(model: &str) -> Result<&str> {
    let model = model.trim();
    if model.is_empty() {
        return Err(error::invalid_request("model requires non-empty text"));
    }

    Ok(model)
}

/// Normalizes an optional conversation reasoning effort before persistence.
fn normalize_conversation_reasoning_effort(effort: Option<&str>) -> Option<String> {
    effort
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .map(str::to_string)
}
