//! Messages persistence owned by the store module.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum InsertSelection<'a> {
    Always,
    IfCurrent(Option<&'a MessageId>),
}

struct MultipartInsert<'a> {
    role: Role,
    content: &'a str,
    parts: &'a [UnsavedMessagePart],
    metadata: Option<&'a MessageMetadata>,
    selection: InsertSelection<'a>,
}

impl Store {
    /// Sets the active message ID for one conversation.
    pub fn set_active_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start active message transaction")?;

        set_active_message_in_transaction(&transaction, conversation_id, Some(message_id))
            .context("failed to set active message")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit active message update")?;

        Ok(())
    }

    /// Loads the selected root-to-active path for one conversation.
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

    /// Inserts a new message and updates the conversation timestamp in one
    /// transaction.
    ///
    /// If a parent message is provided, the new message becomes that parent's
    /// child. The parent must belong to the same conversation.
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

    /// Replaces message content without deleting later messages.
    ///
    /// Existing compactions are cleared because changing earlier text can make
    /// saved summaries incorrect.
    pub fn replace_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        content: &str,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start message update transaction")?;

        transaction
            .execute(
                "UPDATE messages SET content = ?1 WHERE conversation_id = ?2 AND id = ?3",
                params![content, conversation_id.as_str(), message_id.as_str()],
            )
            .context("failed to update message")?;
        replace_text_parts_in_transaction(&transaction, message_id, content)
            .context("failed to update message parts")?;
        delete_compactions_for_conversation(&transaction, conversation_id)
            .context("failed to delete compactions after message update")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message update")?;

        Ok(())
    }

    /// Removes one message from the tree while preserving later descendants.
    ///
    /// This is a splice delete: direct children of the removed message are
    /// reparented to the removed message's parent. Descendants below those
    /// children keep their existing parents. If the removed message is a
    /// tool-call assistant or tool-result node, the assistant tool-call group is
    /// deleted together so model context cannot contain dangling tool calls or
    /// dangling tool results.
    ///
    /// A tool-call group is one assistant message with tool-call metadata plus
    /// the linear `role: tool` result chain below it. Deleting either the
    /// assistant tool-call message or any tool-output message in that chain
    /// deletes the whole group, then splices surviving descendants to the
    /// assistant's parent.
    pub fn remove_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;

        let splice_delete = self.message_splice_delete(conversation_id, message_id)?;
        let active_message_id = self.active_message_id(conversation_id)?;
        let next_active_message_id =
            if active_message_id.as_ref().is_some_and(|active_message_id| {
                splice_delete
                    .deleted_message_ids
                    .contains(active_message_id.as_str())
            }) {
                splice_delete
                    .splice_parent_message_id
                    .clone()
                    .or_else(|| splice_delete.promoted_child_ids.first().cloned())
            } else {
                active_message_id
            };
        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start message delete transaction")?;

        delete_compactions_for_conversation(&transaction, conversation_id)
            .context("failed to delete compactions after message delete")?;
        set_active_message_in_transaction(
            &transaction,
            conversation_id,
            next_active_message_id.as_ref(),
        )
        .context("failed to update active message after delete")?;
        for child_id in &splice_delete.promoted_child_ids {
            transaction
                .execute(
                    "
                    UPDATE messages
                    SET parent_message_id = ?1
                    WHERE conversation_id = ?2 AND id = ?3
                    ",
                    params![
                        splice_delete
                            .splice_parent_message_id
                            .as_ref()
                            .map(MessageId::as_str),
                        conversation_id.as_str(),
                        child_id.as_str()
                    ],
                )
                .context("failed to reparent message child during splice delete")?;
        }

        let placeholders = std::iter::repeat_n("?", splice_delete.deleted_message_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "
            DELETE FROM messages
            WHERE conversation_id = ?
              AND id IN ({placeholders})
            "
        );
        let mut delete_params = Vec::with_capacity(splice_delete.deleted_message_ids.len() + 1);
        delete_params.push(conversation_id.as_str().to_string());
        delete_params.extend(splice_delete.deleted_message_ids.iter().cloned());
        transaction
            .execute(&sql, params_from_iter(delete_params))
            .context("failed to delete spliced message")?;
        delete_orphan_image_assets_in_transaction(&transaction)
            .context("failed to delete orphan image assets")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message delete")?;

        Ok(())
    }

    /// Computes the exact message IDs and child promotions for splice delete.
    ///
    /// This is intentionally built before the transaction because all validation
    /// happens against the current tree shape. The transaction then applies only
    /// the already-decided link updates and deletes.
    fn message_splice_delete(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<MessageSpliceDelete> {
        let target = self
            .load_message_tree_row(conversation_id, message_id)
            .context("failed to load target message")?;

        let (splice_parent_message_id, deleted_message_ids) = match target.role {
            Role::Assistant if !assistant_tool_calls(&target).is_empty() => {
                let deleted_message_ids =
                    self.assistant_tool_group_message_ids(conversation_id, &target)?;
                (target.parent_message_id.clone(), deleted_message_ids)
            }
            Role::Tool => {
                let tool_call_id = target
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.tool_call_id.as_ref())
                    .ok_or_else(|| {
                        error::invalid_request(
                            "cannot remove role: tool message without a tool_call_id",
                        )
                    })?;
                let assistant = self.assistant_tool_group_owner(conversation_id, &target)?;

                let parent_tool_calls = assistant_tool_calls(&assistant);
                if !parent_tool_calls.contains(tool_call_id) {
                    return Err(error::invalid_request(
                        "cannot remove role: tool message because it does not match a parent assistant tool call",
                    ));
                }

                (
                    assistant.parent_message_id.clone(),
                    self.assistant_tool_group_message_ids(conversation_id, &assistant)?,
                )
            }
            _ => (
                target.parent_message_id.clone(),
                HashSet::from([target.id.as_str().to_string()]),
            ),
        };

        let promoted_child_ids = self
            .direct_child_ids_for_removed_messages(conversation_id, &deleted_message_ids)
            .context("failed to load promoted message children")?;

        Ok(MessageSpliceDelete {
            deleted_message_ids,
            splice_parent_message_id,
            promoted_child_ids,
        })
    }

    /// Deletes descendant messages below a checkpoint message in one transaction.
    ///
    /// Compactions are cleared because the visible history changed.
    pub fn truncate_after_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        let descendant_ids = self
            .descendant_message_ids(conversation_id, message_id, false)
            .context("failed to load message descendants")?;
        let active_message_id = self.active_message_id(conversation_id)?;
        let next_active_message_id = if active_message_id
            .as_ref()
            .is_some_and(|active_message_id| descendant_ids.contains(active_message_id.as_str()))
        {
            Some(message_id.clone())
        } else {
            active_message_id
        };

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation truncate transaction")?;

        delete_compactions_for_conversation(&transaction, conversation_id)
            .context("failed to delete compactions after conversation truncate")?;
        set_active_message_in_transaction(
            &transaction,
            conversation_id,
            next_active_message_id.as_ref(),
        )
        .context("failed to update active message after truncate")?;
        transaction
            .execute(
                "
                WITH RECURSIVE subtree(id) AS (
                    SELECT messages.id
                    FROM messages
                    WHERE messages.conversation_id = ?1
                      AND messages.parent_message_id = ?2
                    UNION ALL
                    SELECT messages.id
                    FROM messages
                    JOIN subtree ON messages.parent_message_id = subtree.id
                    WHERE messages.conversation_id = ?1
                )
                DELETE FROM messages
                WHERE conversation_id = ?1
                  AND id IN (SELECT id FROM subtree)
                ",
                params![conversation_id.as_str(), message_id.as_str()],
            )
            .context("failed to prune conversation descendants")?;
        delete_orphan_image_assets_in_transaction(&transaction)
            .context("failed to delete orphan image assets")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit conversation truncate")?;

        Ok(())
    }

    /// Creates a new conversation copied from the source conversation through a
    /// checkpoint message.
    ///
    /// Messages receive new IDs in the fork so both conversations can diverge
    /// independently after creation.
    pub fn fork_conversation_at_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<ConversationId> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;

        let source_messages = self
            .load_path_to_message(conversation_id, message_id)
            .context("failed to load messages for conversation fork")?;
        let source_model = self.conversation_model(conversation_id)?;
        let source_reasoning_effort = self.conversation_reasoning_effort(conversation_id)?;
        let source_tool_approval_mode = self.tool_approval_mode(conversation_id)?;
        let forked_conversation_id = ConversationId::new(Uuid::new_v4().to_string());
        let mut message_id_map = HashMap::new();
        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation fork transaction")?;

        transaction
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
                VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, ?5)
                ",
                params![
                    forked_conversation_id.as_str(),
                    source_model,
                    source_reasoning_effort,
                    source_tool_approval_mode.as_storage(),
                    now
                ],
            )
            .context("failed to create forked conversation")?;

        for (index, message) in source_messages.iter().enumerate() {
            let source_message_id = message
                .id
                .as_ref()
                .ok_or_else(|| anyhow!("stored message is missing id"))?;
            let forked_message_id = MessageId::new(Uuid::new_v4().to_string());
            let forked_parent_message_id = message
                .parent_message_id
                .as_ref()
                .and_then(|parent_message_id| message_id_map.get(parent_message_id.as_str()));
            let metadata_json = encode_message_metadata(message.metadata.as_ref())?;

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
                        forked_message_id.as_str(),
                        forked_conversation_id.as_str(),
                        forked_parent_message_id.map(MessageId::as_str),
                        message.role.as_str(),
                        message.content.as_str(),
                        metadata_json.as_deref(),
                        now + index as i64
                    ],
                )
                .context("failed to copy forked conversation message")?;
            insert_message_parts_in_transaction(
                &transaction,
                &forked_message_id,
                &message.parts,
                now + index as i64,
            )
            .context("failed to copy forked conversation message parts")?;

            message_id_map.insert(source_message_id.as_str().to_string(), forked_message_id);
        }

        let forked_active_message_id = source_messages
            .last()
            .and_then(|message| message.id.as_ref())
            .and_then(|message_id| message_id_map.get(message_id.as_str()));
        set_active_message_in_transaction(
            &transaction,
            &forked_conversation_id,
            forked_active_message_id,
        )
        .context("failed to set forked active message")?;
        transaction
            .commit()
            .context("failed to commit conversation fork")?;

        Ok(forked_conversation_id)
    }
}

pub(super) fn select_inserted_message(
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

/// Decodes message roles from SQLite into the typed runtime role.
impl FromSql for Role {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            role => Err(FromSqlError::Other(
                format!("unknown message role: {role}").into(),
            )),
        }
    }
}

#[derive(Debug, Clone)]
/// Minimal persisted message facts used to mutate tree links.
///
/// Full message loading attaches message parts and content for runtime use. Tree
/// mutation only needs identity, parent links, role, metadata, and stable
/// insertion order, so this row keeps delete planning small and explicit.
struct MessageTreeRow {
    id: MessageId,
    parent_message_id: Option<MessageId>,
    role: Role,
    metadata: Option<MessageMetadata>,
}

#[derive(Debug, Clone)]
/// Concrete splice delete operation computed before the transaction starts.
struct MessageSpliceDelete {
    deleted_message_ids: HashSet<String>,
    splice_parent_message_id: Option<MessageId>,
    promoted_child_ids: Vec<MessageId>,
}

impl Store {
    /// Loads one message row with the fields needed for tree mutation.
    fn load_message_tree_row(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<MessageTreeRow> {
        self.connection
            .query_row(
                "
                SELECT id, parent_message_id, role, metadata
                FROM messages
                WHERE conversation_id = ?1 AND id = ?2
                ",
                params![conversation_id.as_str(), message_id.as_str()],
                read_message_tree_row,
            )
            .optional()
            .context("failed to load message tree row")?
            .ok_or_else(|| {
                error::not_found(format!(
                    "message does not exist in conversation: {message_id}"
                ))
            })
    }

    /// Finds the assistant that owns a role:tool result chain.
    ///
    /// Tool results are stored linearly: assistant tool-call message, first
    /// result, second result, and so on. Starting from any result in that chain,
    /// walking through `role: tool` parents must eventually reach the assistant
    /// tool-call message.
    fn assistant_tool_group_owner(
        &self,
        conversation_id: &ConversationId,
        tool_result: &MessageTreeRow,
    ) -> Result<MessageTreeRow> {
        let mut parent_message_id = tool_result.parent_message_id.clone().ok_or_else(|| {
            error::invalid_request("cannot remove role: tool message without an assistant parent")
        })?;

        loop {
            let parent = self.load_message_tree_row(conversation_id, &parent_message_id)?;
            match parent.role {
                Role::Assistant if !assistant_tool_calls(&parent).is_empty() => return Ok(parent),
                Role::Tool => {
                    parent_message_id = parent.parent_message_id.clone().ok_or_else(|| {
                        error::invalid_request(
                            "cannot remove role: tool message without an assistant parent",
                        )
                    })?;
                }
                _ => {
                    return Err(error::invalid_request(
                        "cannot remove role: tool message because its parent is not an assistant tool-call message",
                    ));
                }
            }
        }
    }

    /// Verifies that a new tool result answers an assistant-requested tool call.
    ///
    /// The parent may be the assistant tool-call message itself, or a previous
    /// `role: tool` result in the same linear result chain. In both cases the
    /// owning assistant must have requested the provider tool-call ID being
    /// stored.
    pub(super) fn ensure_tool_result_parent_matches_call(
        &self,
        conversation_id: &ConversationId,
        parent_message_id: &MessageId,
        tool_call_id: &ToolCallId,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let parent = self.load_message_tree_row(conversation_id, parent_message_id)?;
        let assistant = match parent.role {
            Role::Assistant if !assistant_tool_calls(&parent).is_empty() => parent,
            Role::Tool => self.assistant_tool_group_owner(conversation_id, &parent)?,
            _ => {
                return Err(error::invalid_request(
                    "role: tool result parent must be an assistant tool-call message or tool result chain",
                ));
            }
        };

        if !assistant_tool_calls(&assistant).contains(tool_call_id) {
            return Err(error::invalid_request(format!(
                "assistant did not request tool call: {tool_call_id}"
            )));
        }

        Ok(())
    }

    /// Returns the assistant tool-call group deleted as one model-context unit.
    ///
    /// The assistant message owns the tool-call metadata. The persisted tree
    /// relationship is the group boundary: the linear `role: tool` chain below
    /// that assistant is treated as output for the assistant's tool calls and is
    /// deleted with it. Deleting any tool-output message in that chain therefore
    /// removes the parent assistant call and every result in the chain.
    fn assistant_tool_group_message_ids(
        &self,
        conversation_id: &ConversationId,
        assistant: &MessageTreeRow,
    ) -> Result<HashSet<String>> {
        let mut deleted_message_ids = HashSet::from([assistant.id.as_str().to_string()]);
        let mut stack = vec![assistant.id.clone()];

        while let Some(parent_id) = stack.pop() {
            for tool_result in self.direct_tool_result_children(conversation_id, &parent_id)? {
                if deleted_message_ids.insert(tool_result.id.as_str().to_string()) {
                    stack.push(tool_result.id);
                }
            }
        }

        Ok(deleted_message_ids)
    }

    /// Loads immediate role:tool children while walking a linear tool-result chain.
    fn direct_tool_result_children(
        &self,
        conversation_id: &ConversationId,
        parent_message_id: &MessageId,
    ) -> Result<Vec<MessageTreeRow>> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_message_id, role, metadata
                FROM messages
                WHERE conversation_id = ?1
                  AND role = 'tool'
                  AND parent_message_id = ?2
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare assistant tool group load")?;
        statement
            .query_map(
                params![conversation_id.as_str(), parent_message_id.as_str()],
                read_message_tree_row,
            )
            .context("failed to load assistant tool group rows")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read assistant tool group rows")
    }

    /// Loads direct children of removed messages that must be promoted.
    ///
    /// Children are returned in stable insertion order. Children that are also
    /// being deleted, such as the tool-result child in a tool pair, are skipped.
    fn direct_child_ids_for_removed_messages(
        &self,
        conversation_id: &ConversationId,
        deleted_message_ids: &HashSet<String>,
    ) -> Result<Vec<MessageId>> {
        let placeholders = std::iter::repeat_n("?", deleted_message_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "
            SELECT id
            FROM messages
            WHERE conversation_id = ?
              AND parent_message_id IN ({placeholders})
            ORDER BY created_at, rowid
            "
        );
        let mut query_params = Vec::with_capacity(deleted_message_ids.len() + 1);
        query_params.push(conversation_id.as_str().to_string());
        query_params.extend(deleted_message_ids.iter().cloned());

        let mut statement = self
            .connection
            .prepare(&sql)
            .context("failed to prepare direct child load")?;
        let child_ids = statement
            .query_map(params_from_iter(query_params), |row| {
                row.get::<_, String>(0)
            })
            .context("failed to load direct children")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read direct children")?
            .into_iter()
            .filter(|child_id| !deleted_message_ids.contains(child_id))
            .map(MessageId::new)
            .collect();

        Ok(child_ids)
    }

    /// Loads descendant message IDs below one message in the conversation tree.
    fn descendant_message_ids(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        include_self: bool,
    ) -> Result<HashSet<String>> {
        let seed = if include_self {
            "
            SELECT ?2
            "
        } else {
            "
            SELECT messages.id
            FROM messages
            WHERE messages.conversation_id = ?1
              AND messages.parent_message_id = ?2
            "
        };
        let sql = format!(
            "
            WITH RECURSIVE subtree(id) AS (
                {seed}
                UNION ALL
                SELECT messages.id
                FROM messages
                JOIN subtree ON messages.parent_message_id = subtree.id
                WHERE messages.conversation_id = ?1
            )
            SELECT id FROM subtree
            "
        );
        let mut statement = self
            .connection
            .prepare(&sql)
            .context("failed to prepare descendant load")?;
        let ids = statement
            .query_map(
                params![conversation_id.as_str(), message_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .context("failed to load descendants")?
            .collect::<rusqlite::Result<HashSet<_>>>()
            .context("failed to read descendants")?;

        Ok(ids)
    }

    /// Finds which conversation owns a message ID.
    fn message_conversation_id(&self, message_id: &MessageId) -> Result<Option<ConversationId>> {
        self.connection
            .query_row(
                "SELECT conversation_id FROM messages WHERE id = ?1",
                params![message_id.as_str()],
                |row| Ok(ConversationId::new(row.get::<_, String>(0)?)),
            )
            .optional()
            .context("failed to load message conversation")
    }

    /// Enforces the store boundary that message-scoped operations cannot cross
    /// conversations.
    pub(super) fn ensure_message_belongs_to_conversation(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        let message_conversation_id = self
            .message_conversation_id(message_id)?
            .ok_or_else(|| error::not_found(format!("message does not exist: {message_id}")))?;

        if message_conversation_id != *conversation_id {
            return Err(error::invalid_request(format!(
                "message does not belong to conversation: {message_id}"
            )));
        }

        Ok(())
    }
}

fn read_message_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    let metadata_json = row.get::<_, Option<String>>(4)?;

    Ok(Message {
        id: Some(MessageId::new(row.get::<_, String>(0)?)),
        parent_message_id: row.get::<_, Option<String>>(1)?.map(MessageId::new),
        role: row.get(2)?,
        content: row.get(3)?,
        parts: Vec::new(),
        metadata: decode_message_metadata(metadata_json)?,
    })
}

/// Converts one SQLite message row into a lightweight tree mutation row.
fn read_message_tree_row(row: &Row<'_>) -> rusqlite::Result<MessageTreeRow> {
    let metadata_json = row.get::<_, Option<String>>(3)?;

    Ok(MessageTreeRow {
        id: MessageId::new(row.get::<_, String>(0)?),
        parent_message_id: row.get::<_, Option<String>>(1)?.map(MessageId::new),
        role: row.get(2)?,
        metadata: decode_message_metadata(metadata_json)?,
    })
}

/// Returns assistant tool-call IDs from message metadata.
fn assistant_tool_calls(message: &MessageTreeRow) -> Vec<ToolCallId> {
    message
        .metadata
        .as_ref()
        .map(|metadata| {
            metadata
                .tool_calls
                .iter()
                .map(|tool_call| tool_call.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Converts one SQLite message part row into the runtime message part type.
fn read_message_part_row(row: &Row<'_>) -> rusqlite::Result<(String, MessagePart)> {
    let message_id = row.get::<_, String>(0)?;
    let kind = row.get::<_, String>(1)?;
    let part = match kind.as_str() {
        "text" => MessagePart::Text(row.get::<_, String>(2)?),
        "image" => MessagePart::Image(ImagePart {
            asset_id: ImageAssetId::new(row.get::<_, String>(3)?),
            mime_type: row.get(4)?,
            bytes: row.get(5)?,
        }),
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                Type::Text,
                format!("unknown message part kind: {kind}").into(),
            ));
        }
    };

    Ok((message_id, part))
}

/// Serializes typed message metadata for SQLite storage.
pub(super) fn encode_message_metadata(
    metadata: Option<&MessageMetadata>,
) -> Result<Option<String>> {
    metadata
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize message metadata")
}

/// Decodes SQLite JSON metadata into the typed runtime metadata model.
fn decode_message_metadata(metadata: Option<String>) -> rusqlite::Result<Option<MessageMetadata>> {
    metadata
        .map(|metadata| {
            serde_json::from_str(&metadata).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })
        })
        .transpose()
}

/// Serializes a tool's JSON schema parameters for SQLite storage.
pub(super) fn insert_unsaved_message_parts_in_transaction(
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
fn insert_message_parts_in_transaction(
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
fn insert_text_part_in_transaction(
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

/// Replaces text parts when a message uses ordered model-facing parts.
///
/// Plain text-only messages have no `message_parts` rows, so their updated
/// `messages.content` value is already the single source of truth. Multimodal
/// messages keep image parts and refresh the leading text part to match the
/// updated preview content.
fn replace_text_parts_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    content: &str,
) -> Result<()> {
    let part_count = transaction
        .query_row(
            "SELECT COUNT(*) FROM message_parts WHERE message_id = ?1",
            params![message_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .context("failed to count message parts")?;
    if part_count == 0 {
        return Ok(());
    }

    transaction
        .execute(
            "DELETE FROM message_parts WHERE message_id = ?1 AND kind = 'text'",
            params![message_id.as_str()],
        )
        .context("failed to delete old text message parts")?;

    let image_start_position = if content.is_empty() { 0 } else { 1 };
    normalize_message_part_positions_in_transaction(transaction, message_id, image_start_position)
        .context("failed to normalize message part positions")?;

    if !content.is_empty() {
        insert_text_part_in_transaction(transaction, message_id, 0, content)
            .context("failed to save updated text message part")?;
    }

    Ok(())
}

/// Rewrites remaining message part positions into a dense ordered range.
fn normalize_message_part_positions_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    start_position: usize,
) -> Result<()> {
    let part_ids = {
        let mut statement = transaction
            .prepare(
                "
                SELECT id
                FROM message_parts
                WHERE message_id = ?1
                ORDER BY position, rowid
                ",
            )
            .context("failed to prepare message part position load")?;
        statement
            .query_map(params![message_id.as_str()], |row| row.get::<_, String>(0))
            .context("failed to load message part positions")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read message part positions")?
    };

    for (index, part_id) in part_ids.iter().enumerate() {
        transaction
            .execute(
                "UPDATE message_parts SET position = ?1 WHERE id = ?2",
                params![(start_position + index) as i64, part_id],
            )
            .context("failed to update message part position")?;
    }

    Ok(())
}

/// Deletes all compaction checkpoints for a conversation inside an existing
/// transaction.
fn delete_compactions_for_conversation(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
) -> Result<()> {
    transaction.execute(
        "DELETE FROM compactions WHERE conversation_id = ?1",
        params![conversation_id.as_str()],
    )?;

    Ok(())
}
fn set_active_message_in_transaction(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    message_id: Option<&MessageId>,
) -> Result<()> {
    transaction.execute(
        "UPDATE conversations SET active_message_id = ?1 WHERE id = ?2",
        params![message_id.map(MessageId::as_str), conversation_id.as_str()],
    )?;

    Ok(())
}
