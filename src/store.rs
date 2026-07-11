//! SQLite persistence boundary.
//!
//! This facade owns shared transactions and conversation-tree integrity.
//! Focused child modules own schema, run, conversation, message, tool, image,
//! and compaction queries. Code outside `store` should not know SQLite tables.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, Type, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::conversation::{
    CompactionId, ConversationId, ImageAssetId, ImagePart, Message, MessageId, MessageMetadata,
    MessagePart, Role, ToolCallId, ToolSchema, ToolSchemaName, UnsavedMessagePart,
};
use crate::error;
use crate::paths;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolApprovalMode, ToolPermission,
    ToolProviderId, ToolProviderKind, ToolProviderRef,
};

mod compactions;
mod conversations;
mod images;
mod messages;
mod runs;
mod schema;
mod tools;

use images::{
    delete_orphan_image_assets_in_transaction, insert_image_asset_in_transaction,
    insert_image_part_in_transaction,
};

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

#[cfg(test)]
const DEFAULT_CONVERSATION_ID: &str = "default";
const DATABASE_SCHEMA_VERSION: i32 = 11;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Lightweight row used by conversation listing.
pub struct ConversationInfo {
    pub id: ConversationId,
    pub model: String,
    pub message_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Saved summary of conversation history through a specific message.
pub struct Compaction {
    pub id: CompactionId,
    pub conversation_id: ConversationId,
    pub through_message_id: MessageId,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Persisted backend-owned runtime run.
pub struct RuntimeRunRecord {
    pub id: String,
    pub conversation_id: ConversationId,
    pub status: RuntimeRunStatus,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Persisted lifecycle state for a backend-owned runtime run.
pub enum RuntimeRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RuntimeRunStatus {
    /// Returns the SQLite representation for this status.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    /// Decodes a persisted runtime run status.
    fn from_storage(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

impl std::fmt::Display for RuntimeRunStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

fn read_runtime_run_row(row: &Row<'_>) -> rusqlite::Result<RuntimeRunRecord> {
    let status = row.get::<_, String>(2)?;
    let status = RuntimeRunStatus::from_storage(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            format!("unknown runtime run status: {status}").into(),
        )
    })?;

    Ok(RuntimeRunRecord {
        id: row.get(0)?,
        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
        status,
        error: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One ordered serialized event emitted by a runtime run.
pub struct RuntimeRunEventRecord {
    pub sequence: u64,
    pub payload: String,
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

/// SQLite-backed persistence boundary for conversations, messages, and
/// compactions.
pub struct Store {
    connection: Connection,
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
    fn ensure_tool_result_parent_matches_call(
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
    fn ensure_message_belongs_to_conversation(
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

    /// Returns an error instead of silently treating missing conversations as
    /// empty.
    fn ensure_conversation_exists(&self, conversation_id: &ConversationId) -> Result<()> {
        if !self.conversation_exists(conversation_id)? {
            return Err(error::not_found(format!(
                "conversation does not exist: {conversation_id}"
            )));
        }

        Ok(())
    }

    /// Returns an error when a tool schema name is not present on the
    /// conversation being mutated.
    fn ensure_tool_schema_exists(
        &self,
        conversation_id: &ConversationId,
        name: &ToolSchemaName,
    ) -> Result<()> {
        let exists = self
            .connection
            .query_row(
                "
                SELECT 1
                FROM tool_schemas
                WHERE conversation_id = ?1 AND name = ?2
                ",
                params![conversation_id.as_str(), name.as_str()],
                |_| Ok(()),
            )
            .optional()
            .context("failed to check tool schema")?
            .is_some();

        if !exists {
            return Err(error::not_found(format!(
                "tool schema does not exist: {name}"
            )));
        }

        Ok(())
    }

    /// Checks whether one conversation row exists.
    fn conversation_exists(&self, conversation_id: &ConversationId) -> Result<bool> {
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

    /// Checks whether one SQLite table already exists.
    fn table_exists(&self, table_name: &str) -> Result<bool> {
        let exists = self
            .connection
            .query_row(
                "
                SELECT 1
                FROM sqlite_master
                WHERE type = 'table' AND name = ?1
                ",
                params![table_name],
                |_| Ok(()),
            )
            .optional()
            .context("failed to inspect database tables")?
            .is_some();

        Ok(exists)
    }
}

/// Builds the default user database path.
fn default_database_path() -> Result<PathBuf> {
    Ok(paths::database_path())
}

/// Converts one SQLite message row into the runtime message type.
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

/// Converts one SQLite attached tool row into the runtime attachment type.
fn read_attached_tool_row(row: &Row<'_>) -> rusqlite::Result<AttachedTool> {
    let parameters_json = row.get::<_, String>(2)?;
    let parameters = serde_json::from_str(&parameters_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
    })?;
    let provider_kind_text = row.get::<_, String>(5)?;
    let provider_kind = ToolProviderKind::from_storage(&provider_kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            Type::Text,
            format!("unknown tool provider kind: {provider_kind_text}").into(),
        )
    })?;
    let permissions_json = row.get::<_, String>(6)?;
    let permissions =
        serde_json::from_str::<Vec<ToolPermission>>(&permissions_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
        })?;
    let annotations_json = row.get::<_, String>(7)?;
    let annotations =
        serde_json::from_str::<ToolAnnotations>(&annotations_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(error))
        })?;

    Ok(AttachedTool {
        schema_name: ToolSchemaName::new(row.get::<_, String>(0)?),
        description: row.get(1)?,
        parameters,
        provider: ToolProviderRef::new(
            ToolProviderId::new(row.get::<_, String>(3)?),
            ProviderToolName::new(row.get::<_, String>(4)?),
            provider_kind,
        ),
        permissions,
        annotations,
    })
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
fn encode_message_metadata(metadata: Option<&MessageMetadata>) -> Result<Option<String>> {
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
fn encode_tool_parameters(parameters: &serde_json::Value) -> Result<String> {
    if !parameters.is_object() {
        return Err(error::invalid_request(
            "tool schema parameters must be a JSON object",
        ));
    }

    serde_json::to_string(parameters).context("failed to serialize tool schema parameters")
}

/// Serializes attached tool permissions for SQLite storage.
fn encode_tool_permissions(permissions: &[ToolPermission]) -> Result<String> {
    serde_json::to_string(permissions).context("failed to serialize tool permissions")
}

/// Serializes attached tool annotations for SQLite storage.
fn encode_tool_annotations(annotations: &ToolAnnotations) -> Result<String> {
    serde_json::to_string(annotations).context("failed to serialize tool annotations")
}

/// Validates the attached tool contract before storing it.
fn validate_attached_tool(attached_tool: &AttachedTool) -> Result<()> {
    if !attached_tool.schema_name.is_valid() {
        return Err(error::invalid_request(
            "tool schema name must be 1-64 characters using letters, numbers, '_', or '-'",
        ));
    }
    if attached_tool.description.trim().is_empty() {
        return Err(error::invalid_request(
            "tool schema description must not be empty",
        ));
    }
    if !attached_tool.parameters.is_object() {
        return Err(error::invalid_request(
            "tool schema parameters must be a JSON object",
        ));
    }

    Ok(())
}

/// Inserts one already-validated attached tool inside an existing transaction.
fn insert_attached_tool_in_transaction(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    attached_tool: &AttachedTool,
    now: i64,
) -> Result<()> {
    let parameters_json = encode_tool_parameters(&attached_tool.parameters)?;
    let permissions_json = encode_tool_permissions(&attached_tool.permissions)?;
    let annotations_json = encode_tool_annotations(&attached_tool.annotations)?;

    transaction.execute(
        "
        INSERT INTO tool_schemas (
            conversation_id,
            name,
            description,
            parameters_json,
            provider_id,
            provider_tool_name,
            provider_kind,
            permissions_json,
            annotations_json,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
        ",
        params![
            conversation_id.as_str(),
            attached_tool.schema_name.as_str(),
            attached_tool.description.as_str(),
            parameters_json.as_str(),
            attached_tool.provider.provider_id.as_str(),
            attached_tool.provider.tool_name.as_str(),
            attached_tool.provider.kind.as_storage(),
            permissions_json.as_str(),
            annotations_json.as_str(),
            now
        ],
    )?;

    Ok(())
}

/// Writes all ordered unsaved parts for one new message into an existing
/// transaction.
fn insert_unsaved_message_parts_in_transaction(
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

/// Updates conversation ordering metadata inside an existing transaction.
fn touch_conversation_in_transaction(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    updated_at: i64,
) -> Result<()> {
    transaction.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![updated_at, conversation_id.as_str()],
    )?;

    Ok(())
}

/// Updates the selected active message for a conversation inside an existing
/// transaction.
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

/// Returns current Unix time in milliseconds for ordering persisted rows.
fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
mod tests;
