//! Message insertion and selection.

use super::super::{
    Context, ConversationId, MessageId, MessageMetadata, MessagePart, Result, Role, Store,
    ToolCallId, Transaction, UnsavedMessagePart, Uuid, error, insert_image_asset_in_transaction,
    insert_image_part_in_transaction, now_millis, params, touch_conversation_in_transaction,
};
use super::InsertSelection;
use super::codecs::encode_message_metadata;
use super::mutate::set_active_message_in_transaction;

struct MultipartInsert<'a> {
    role: Role,
    content: &'a str,
    parts: &'a [UnsavedMessagePart],
    metadata: Option<&'a MessageMetadata>,
    selection: InsertSelection<'a>,
}

impl Store {
    pub fn insert_message(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        role: Role,
        content: &str,
        metadata: Option<&MessageMetadata>,
    ) -> Result<MessageId> {
        if role == Role::Tool {
            return Err(error::invalid_request(
                "role: tool messages must be created through insert_tool_result_message",
            ));
        }

        self.insert_message_unchecked(
            conversation_id,
            parent_message_id,
            role,
            content,
            metadata,
            InsertSelection::Always,
        )
    }

    /// Appends an assistant response to its captured branch. The response only
    /// becomes active when the user has not selected another path meanwhile.
    pub fn insert_assistant_message_on_branch(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        content: &str,
        metadata: Option<&MessageMetadata>,
    ) -> Result<MessageId> {
        self.insert_message_unchecked(
            conversation_id,
            parent_message_id,
            Role::Assistant,
            content,
            metadata,
            InsertSelection::IfCurrent(parent_message_id),
        )
    }

    /// Inserts one tool result message after validating the assistant tool-call
    /// chain it answers.
    ///
    /// Generic message insertion cannot create `role: tool` messages. Runtime
    /// must use this primitive so the store can enforce that every tool result
    /// is linked to a provider tool-call ID requested by an assistant message in
    /// the same conversation path.
    pub fn insert_tool_result_message(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: &MessageId,
        tool_call_id: &ToolCallId,
        content: &str,
    ) -> Result<MessageId> {
        self.ensure_tool_result_parent_matches_call(
            conversation_id,
            parent_message_id,
            tool_call_id,
        )?;
        let metadata = MessageMetadata {
            tool_call_id: Some(tool_call_id.clone()),
            ..Default::default()
        };

        self.insert_message_unchecked(
            conversation_id,
            Some(parent_message_id),
            Role::Tool,
            content,
            Some(&metadata),
            InsertSelection::Always,
        )
    }

    /// Appends a tool result to the execution branch without overriding a path
    /// the user selected while the external tool was running.
    #[cfg(test)]
    pub fn insert_tool_result_message_on_branch(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: &MessageId,
        tool_call_id: &ToolCallId,
        content: &str,
        parts: &[UnsavedMessagePart],
    ) -> Result<MessageId> {
        self.ensure_tool_result_parent_matches_call(
            conversation_id,
            parent_message_id,
            tool_call_id,
        )?;
        let metadata = MessageMetadata {
            tool_call_id: Some(tool_call_id.clone()),
            ..Default::default()
        };
        let selection = InsertSelection::IfCurrent(Some(parent_message_id));
        if parts.is_empty() {
            self.insert_message_unchecked(
                conversation_id,
                Some(parent_message_id),
                Role::Tool,
                content,
                Some(&metadata),
                selection,
            )
        } else {
            self.insert_message_with_parts_unchecked(
                conversation_id,
                Some(parent_message_id),
                MultipartInsert {
                    role: Role::Tool,
                    content,
                    parts,
                    metadata: Some(&metadata),
                    selection,
                },
            )
        }
    }

    /// Inserts one message without the public role gate.
    ///
    /// Only store-owned primitives call this helper. Public callers must use
    /// `insert_message` for normal messages or `insert_tool_result_message` for
    /// tool results so role-specific invariants stay centralized here.
    fn insert_message_unchecked(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        role: Role,
        content: &str,
        metadata: Option<&MessageMetadata>,
        selection: InsertSelection<'_>,
    ) -> Result<MessageId> {
        let id = MessageId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;
        let metadata_json = encode_message_metadata(metadata)?;

        self.ensure_conversation_exists(conversation_id)?;

        if let Some(parent_message_id) = parent_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, parent_message_id)?;
        }

        let transaction = self
            .connection
            .transaction()
            .context("failed to start message save transaction")?;

        transaction
            .execute(
                "
                INSERT INTO messages (
                    id,
                    conversation_id,
                    parent_message_id,
                    role,
                    content,
                    metadata,
                    created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
                params![
                    id.as_str(),
                    conversation_id.as_str(),
                    parent_message_id.map(MessageId::as_str),
                    role.as_str(),
                    content,
                    metadata_json.as_deref(),
                    now
                ],
            )
            .context("failed to save message")?;

        select_inserted_message(&transaction, conversation_id, &id, selection)?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message save")?;

        Ok(id)
    }

    /// Inserts a new message with ordered text/image parts.
    ///
    /// This is the shared multipart storage primitive for model-facing
    /// messages. User images and rich tool results both flow through the same
    /// persisted `message_parts` and `image_assets` tables.
    pub fn insert_message_with_parts(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        role: Role,
        content: &str,
        parts: &[UnsavedMessagePart],
        metadata: Option<&MessageMetadata>,
    ) -> Result<MessageId> {
        if role == Role::Tool {
            return Err(error::invalid_request(
                "role: tool messages must be created through a tool-result operation",
            ));
        }

        self.insert_message_with_parts_unchecked(
            conversation_id,
            parent_message_id,
            MultipartInsert {
                role,
                content,
                parts,
                metadata,
                selection: InsertSelection::Always,
            },
        )
    }

    /// Inserts a multipart message without the public role gate.
    fn insert_message_with_parts_unchecked(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        insert: MultipartInsert<'_>,
    ) -> Result<MessageId> {
        if insert.parts.is_empty() {
            return Err(error::invalid_request(
                "message parts require at least one part",
            ));
        }

        let id = MessageId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;
        let metadata_json = encode_message_metadata(insert.metadata)?;

        self.ensure_conversation_exists(conversation_id)?;

        if let Some(parent_message_id) = parent_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, parent_message_id)?;
        }

        let transaction = self
            .connection
            .transaction()
            .context("failed to start multipart message save transaction")?;

        transaction
            .execute(
                "
                INSERT INTO messages (
                    id,
                    conversation_id,
                    parent_message_id,
                    role,
                    content,
                    metadata,
                    created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
                params![
                    id.as_str(),
                    conversation_id.as_str(),
                    parent_message_id.map(MessageId::as_str),
                    insert.role.as_str(),
                    insert.content,
                    metadata_json.as_deref(),
                    now
                ],
            )
            .context("failed to save multipart message")?;

        insert_unsaved_message_parts_in_transaction(&transaction, &id, insert.parts, now)
            .context("failed to save multipart message parts")?;
        select_inserted_message(&transaction, conversation_id, &id, insert.selection)?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit multipart message save")?;

        Ok(id)
    }
}
pub(in crate::store) fn select_inserted_message(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    message_id: &MessageId,
    selection: InsertSelection<'_>,
) -> Result<()> {
    match selection {
        InsertSelection::Always => {
            set_active_message_in_transaction(transaction, conversation_id, Some(message_id))
        }
        InsertSelection::IfCurrent(expected) => transaction
            .execute(
                "
                UPDATE conversations
                SET active_message_id = ?1
                WHERE id = ?2 AND active_message_id IS ?3
                ",
                params![
                    message_id.as_str(),
                    conversation_id.as_str(),
                    expected.map(MessageId::as_str)
                ],
            )
            .map(|_| ())
            .context("failed to conditionally select inserted message"),
    }
}
/// Serializes a tool's JSON schema parameters for SQLite storage.
pub(in crate::store) fn insert_unsaved_message_parts_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    parts: &[UnsavedMessagePart],
    now: i64,
) -> Result<()> {
    for (position, part) in parts.iter().enumerate() {
        match part {
            UnsavedMessagePart::Text(text) => {
                insert_text_part_in_transaction(transaction, message_id, position, text)
                    .context("failed to save text message part")?;
            }
            UnsavedMessagePart::Image(image) => {
                let image_asset_id = insert_image_asset_in_transaction(
                    transaction,
                    &image.mime_type,
                    &image.bytes,
                    now,
                )
                .context("failed to copy image asset")?;
                insert_image_part_in_transaction(
                    transaction,
                    message_id,
                    position,
                    &image_asset_id,
                )
                .context("failed to save image message part")?;
            }
        }
    }

    Ok(())
}

/// Writes all ordered persisted parts for a copied message into an existing
/// transaction.
pub(super) fn insert_message_parts_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    parts: &[MessagePart],
    now: i64,
) -> Result<()> {
    for (position, part) in parts.iter().enumerate() {
        match part {
            MessagePart::Text(text) => {
                insert_text_part_in_transaction(transaction, message_id, position, text)
                    .context("failed to save text message part")?;
            }
            MessagePart::Image(image) => {
                let image_asset_id = insert_image_asset_in_transaction(
                    transaction,
                    &image.mime_type,
                    &image.bytes,
                    now,
                )
                .context("failed to copy image asset")?;
                insert_image_part_in_transaction(
                    transaction,
                    message_id,
                    position,
                    &image_asset_id,
                )
                .context("failed to save image message part")?;
            }
        }
    }

    Ok(())
}

/// Writes one text message part into an existing transaction.
pub(super) fn insert_text_part_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    position: usize,
    text: &str,
) -> Result<()> {
    transaction
        .execute(
            "
            INSERT INTO message_parts (id, message_id, position, kind, text, image_asset_id)
            VALUES (?1, ?2, ?3, 'text', ?4, NULL)
            ",
            params![
                Uuid::new_v4().to_string(),
                message_id.as_str(),
                position as i64,
                text
            ],
        )
        .context("failed to save text message part")?;

    Ok(())
}
