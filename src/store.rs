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
    MessagePart, MessagePartView, MessageView, Role, ToolCallId, ToolSchema, ToolSchemaName,
    UnsavedMessagePart,
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
use messages::{
    InsertSelection, encode_message_metadata, insert_unsaved_message_parts_in_transaction,
    select_inserted_message,
};
use tools::read_attached_tool_row;

#[cfg(test)]
const DEFAULT_CONVERSATION_ID: &str = "default";
const DATABASE_SCHEMA_VERSION: i32 = 15;

pub use compactions::Compaction;
pub use conversations::ConversationInfo;
pub use runs::{RuntimeRunAction, RuntimeRunEventRecord, RuntimeRunRecord, RuntimeRunStatus};
pub use tools::ToolExecutionRecord;
#[cfg(test)]
use tools::ToolExecutionStatus;

/// SQLite-backed persistence boundary for conversations, messages, tools, runs,
/// images, and compactions.
pub struct Store {
    connection: Connection,
}

impl Store {
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

/// Returns current Unix time in milliseconds for ordering persisted rows.
fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
mod tests;
