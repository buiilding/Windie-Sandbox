//! Messages persistence owned by the store module.

use super::*;

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

        self.insert_message_unchecked(conversation_id, parent_message_id, role, content, metadata)
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
        )
    }

    /// Inserts a rich tool result with ordered model-facing parts.
    ///
    /// This is the multipart companion to `insert_tool_result_message`. It is
    /// used by screenshot-like tools that need to persist text and image parts
    /// while preserving the same assistant tool-call ownership invariant.
    pub fn insert_tool_result_message_with_parts(
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

        self.insert_message_with_parts_unchecked(
            conversation_id,
            Some(parent_message_id),
            Role::Tool,
            content,
            parts,
            Some(&metadata),
        )
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

        set_active_message_in_transaction(&transaction, conversation_id, Some(&id))
            .context("failed to set active message")?;
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
                "role: tool messages must be created through insert_tool_result_message_with_parts",
            ));
        }

        self.insert_message_with_parts_unchecked(
            conversation_id,
            parent_message_id,
            role,
            content,
            parts,
            metadata,
        )
    }

    /// Inserts a multipart message without the public role gate.
    fn insert_message_with_parts_unchecked(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        role: Role,
        content: &str,
        parts: &[UnsavedMessagePart],
        metadata: Option<&MessageMetadata>,
    ) -> Result<MessageId> {
        if parts.is_empty() {
            return Err(error::invalid_request(
                "message parts require at least one part",
            ));
        }

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
                    role.as_str(),
                    content,
                    metadata_json.as_deref(),
                    now
                ],
            )
            .context("failed to save multipart message")?;

        insert_unsaved_message_parts_in_transaction(&transaction, &id, parts, now)
            .context("failed to save multipart message parts")?;
        set_active_message_in_transaction(&transaction, conversation_id, Some(&id))
            .context("failed to set active message")?;
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
