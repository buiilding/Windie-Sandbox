//! Message and part loading.

use super::super::*;
use super::codecs::{read_message_part_row, read_message_row};

impl Store {
    pub fn load_active_path(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let Some(message_id) = self.active_message_id(conversation_id)? else {
            return Ok(Vec::new());
        };

        self.load_path_to_message(conversation_id, &message_id)
    }

    pub fn load_active_path_view(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<MessageView>> {
        let Some(message_id) = self.active_message_id(conversation_id)? else {
            return Ok(Vec::new());
        };
        let rows = self.load_path_to_message_rows(conversation_id, &message_id)?;
        self.message_views_from_rows(rows)
    }

    pub fn load_message_tree_view(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<MessageView>> {
        let rows = self.load_message_rows(conversation_id)?;
        self.message_views_from_rows(rows)
    }

    fn message_views_from_rows(&self, rows: Vec<Message>) -> Result<Vec<MessageView>> {
        let mut messages = rows
            .into_iter()
            .map(MessageView::from_message)
            .collect::<Vec<_>>();
        let message_ids = messages
            .iter()
            .filter_map(|message| message.id.clone())
            .collect::<Vec<_>>();
        if message_ids.is_empty() {
            return Ok(messages);
        }
        let placeholders = (1..=message_ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "
            SELECT message_parts.message_id, message_parts.kind, message_parts.text,
                   image_assets.id, image_assets.mime_type, length(image_assets.bytes)
            FROM message_parts
            LEFT JOIN image_assets ON image_assets.id = message_parts.image_asset_id
            WHERE message_parts.message_id IN ({placeholders})
            ORDER BY message_parts.message_id, message_parts.position, message_parts.rowid
            "
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(message_ids.iter()), |row| {
            let message_id = row.get::<_, String>(0)?;
            let kind = row.get::<_, String>(1)?;
            let part = match kind.as_str() {
                "text" => MessagePartView::Text { text: row.get(2)? },
                "image" => MessagePartView::Image {
                    asset_id: row.get(3)?,
                    mime_type: row.get(4)?,
                    byte_count: row.get::<_, i64>(5)?.max(0) as usize,
                },
                _ => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        1,
                        Type::Text,
                        format!("unknown message part kind: {kind}").into(),
                    ));
                }
            };
            Ok((message_id, part))
        })?;
        let mut parts = HashMap::<String, Vec<MessagePartView>>::new();
        for row in rows {
            let (message_id, part) = row?;
            parts.entry(message_id).or_default().push(part);
        }
        for message in &mut messages {
            if let Some(id) = message.id.as_ref() {
                message.parts = parts.remove(id).unwrap_or_default();
            }
        }
        Ok(messages)
    }

    /// Loads the root-to-message path for one message inside a conversation.
    pub fn load_path_to_message(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<Vec<Message>> {
        let mut path = self.load_path_to_message_rows(conversation_id, message_id)?;
        self.attach_message_parts(&mut path)
            .context("failed to load active path parts")?;

        Ok(path)
    }

    /// Loads message rows for one conversation without attaching ordered parts.
    ///
    /// This is crate-private so `perf.rs` can time row loading separately from
    /// part/image attachment without making row loading part of the public store
    /// API.
    pub(crate) fn load_message_rows(
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
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare message load")?;

        statement
            .query_map(params![conversation_id.as_str()], read_message_row)
            .context("failed to load messages")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages")
    }

    /// Loads the root-to-message rows without attaching ordered parts.
    ///
    /// This is crate-private so `perf.rs` can time active-path row loading
    /// separately from active message lookup and part/image attachment without
    /// exposing the primitive outside the crate.
    /// The recursive step starts from the one-row `path` table and uses
    /// `CROSS JOIN` to keep SQLite on primary-key parent lookups even before a
    /// fresh database has planner statistics.
    pub(crate) fn load_path_to_message_rows(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<Vec<Message>> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;

        let mut statement = self
            .connection
            .prepare(
                "
                WITH RECURSIVE path(
                    id,
                    parent_message_id,
                    role,
                    content,
                    metadata,
                    depth
                ) AS (
                    SELECT
                        id,
                        parent_message_id,
                        role,
                        content,
                        metadata,
                        0
                    FROM messages
                    WHERE conversation_id = ?1 AND id = ?2

                    UNION ALL

                    SELECT
                        messages.id,
                        messages.parent_message_id,
                        messages.role,
                        messages.content,
                        messages.metadata,
                        path.depth + 1
                    FROM path
                    CROSS JOIN messages INDEXED BY messages_id_conversation_idx
                        ON messages.id = path.parent_message_id
                    WHERE messages.conversation_id = ?1
                )
                SELECT id, parent_message_id, role, content, metadata
                FROM path
                ORDER BY depth DESC
                ",
            )
            .context("failed to prepare active path load")?;

        statement
            .query_map(
                params![conversation_id.as_str(), message_id.as_str()],
                read_message_row,
            )
            .context("failed to load active path")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read active path")
    }

    /// Attaches ordered text/image parts to already-loaded message rows.
    ///
    /// This is crate-private so `perf.rs` can time part/image attachment
    /// separately from row loading. Callers must pass messages loaded from this
    /// store.
    pub(crate) fn attach_message_parts(&self, messages: &mut [Message]) -> Result<()> {
        let message_ids = messages
            .iter()
            .filter_map(|message| message.id.as_ref())
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        if message_ids.is_empty() {
            return Ok(());
        }

        let placeholders = (1..=message_ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "
            SELECT
                message_parts.message_id,
                message_parts.kind,
                message_parts.text,
                image_assets.id,
                image_assets.mime_type,
                image_assets.bytes
            FROM message_parts
            LEFT JOIN image_assets ON image_assets.id = message_parts.image_asset_id
            WHERE message_parts.message_id IN ({placeholders})
            ORDER BY message_parts.message_id, message_parts.position, message_parts.rowid
            "
        );
        let mut statement = self
            .connection
            .prepare(&sql)
            .context("failed to prepare message part load")?;
        let mut parts_by_message = HashMap::<String, Vec<MessagePart>>::new();
        let parts = statement
            .query_map(params_from_iter(message_ids.iter()), read_message_part_row)
            .context("failed to load message parts")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read message parts")?;

        for (message_id, part) in parts {
            parts_by_message.entry(message_id).or_default().push(part);
        }

        for message in messages {
            let Some(message_id) = message.id.as_ref() else {
                continue;
            };
            message.parts = parts_by_message
                .remove(message_id.as_str())
                .unwrap_or_default();
        }

        Ok(())
    }
}
