//! Conversation compaction checkpoint persistence.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Saved summary of conversation history through a specific message.
pub struct Compaction {
    pub id: CompactionId,
    pub conversation_id: ConversationId,
    pub through_message_id: MessageId,
    pub content: String,
    pub created_at: i64,
}

impl Store {
    /// Loads the newest compaction checkpoint for one conversation.
    pub fn latest_compaction(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<Compaction>> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "
                SELECT id, conversation_id, through_message_id, content, created_at
                FROM compactions
                WHERE conversation_id = ?1
                ORDER BY created_at DESC, rowid DESC
                LIMIT 1
                ",
                params![conversation_id.as_str()],
                |row| {
                    Ok(Compaction {
                        id: CompactionId::new(row.get::<_, String>(0)?),
                        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
                        through_message_id: MessageId::new(row.get::<_, String>(2)?),
                        content: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .context("failed to load latest compaction")
    }

    /// Saves a compaction summary through a message checkpoint.
    ///
    /// This is currently a stored primitive for future automatic compaction; no
    /// CLI command writes compactions yet.
    ///
    /// The checkpoint message must belong to the same conversation.
    #[allow(dead_code)]
    pub fn save_compaction(
        &mut self,
        conversation_id: &ConversationId,
        through_message_id: &MessageId,
        content: &str,
    ) -> Result<CompactionId> {
        let id = CompactionId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;

        self.ensure_conversation_exists(conversation_id)?;

        self.ensure_message_belongs_to_conversation(conversation_id, through_message_id)?;

        let transaction = self
            .connection
            .transaction()
            .context("failed to start compaction save transaction")?;

        transaction
            .execute(
                "
                INSERT INTO compactions (
                    id,
                    conversation_id,
                    through_message_id,
                    content,
                    created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    id.as_str(),
                    conversation_id.as_str(),
                    through_message_id.as_str(),
                    content,
                    now
                ],
            )
            .context("failed to save compaction")?;

        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit compaction save")?;

        Ok(id)
    }
}

/// Deletes all compaction checkpoints for a conversation inside an existing
/// transaction.
pub(super) fn delete_compactions_for_conversation(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
) -> Result<()> {
    transaction.execute(
        "DELETE FROM compactions WHERE conversation_id = ?1",
        params![conversation_id.as_str()],
    )?;

    Ok(())
}
