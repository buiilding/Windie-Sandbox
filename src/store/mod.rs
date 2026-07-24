//! SQLite persistence boundary.
//!
//! This module owns persisted conversations, messages, sessions, compactions,
//! attached tools, and tool schemas. Other modules should not know about
//! SQLite tables or queries.

mod compaction;
mod conversation;
mod message;
mod provider;
mod schema;
mod session;
mod system_prompt;
mod tool_schema;

pub use compaction::Compaction;
pub use conversation::ConversationInfo;
pub use provider::InstalledProvider;

#[cfg(test)]
use schema::DATABASE_SCHEMA_VERSION;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, Type, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::conversation::{
    CompactionId, ConversationId, ImageAssetId, ImagePart, Message, MessageId, MessageMetadata,
    MessagePart, Role, ToolCallId, UnsavedMessagePart,
};
use crate::error;
use crate::llm::ReasoningRequest;
use crate::session::{Session, SessionEvent, SessionEventRecord, SessionId, SessionStatus};
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolApprovalMode, ToolPermission,
    ToolProviderId, ToolProviderKind, ToolProviderRef, ToolSchema, ToolSchemaName,
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

/// SQLite-backed persistence boundary for conversations, messages, and
/// compactions.
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Opens the default user database at `~/.windie/windie.db`.
    pub fn open() -> Result<Self> {
        Self::open_at(default_database_path()?)
    }

    /// Opens a database at a specific path, creating parent directories and
    /// applying schema setup.
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent).context("failed to create database directory")?;
        }

        let store = Self {
            connection: Connection::open(path).context("failed to open database")?,
        };
        store.configure()?;
        store.migrate()?;
        store.optimize()?;

        Ok(store)
    }

    #[cfg(test)]
    /// Opens an in-memory database for isolated tests.
    pub(crate) fn open_memory() -> Result<Self> {
        let store = Self {
            connection: Connection::open_in_memory().context("failed to open memory database")?,
        };
        store.configure()?;
        store.migrate()?;
        store.optimize()?;

        Ok(store)
    }

    /// Applies SQLite settings used by Windie.
    ///
    /// Foreign keys protect relationships, WAL improves normal local write
    /// behavior, and busy timeout makes brief lock contention less fragile.
    fn configure(&self) -> Result<()> {
        self.connection
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;
                ",
            )
            .context("failed to configure database")
    }

    /// Lets SQLite refresh planner statistics when it decides optimization is
    /// useful.
    fn optimize(&self) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA optimize;")
            .context("failed to optimize database")
    }
}

/// Builds the default user database path.
fn default_database_path() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;

    Ok(PathBuf::from(home).join(".windie").join("windie.db"))
}

/// Converts one SQLite message row into the runtime message type.
pub(super) fn read_message_row(row: &Row<'_>) -> rusqlite::Result<Message> {
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
pub(super) fn decode_message_metadata(
    metadata: Option<String>,
) -> rusqlite::Result<Option<MessageMetadata>> {
    metadata
        .map(|metadata| {
            serde_json::from_str(&metadata).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })
        })
        .transpose()
}

/// Removes image assets no remaining message part references.
pub(super) fn delete_orphan_image_assets_in_transaction(
    transaction: &Transaction<'_>,
) -> Result<()> {
    transaction
        .execute(
            "
            DELETE FROM image_assets
            WHERE id NOT IN (
                SELECT image_asset_id
                FROM message_parts
                WHERE image_asset_id IS NOT NULL
            )
            ",
            [],
        )
        .context("failed to delete orphan image assets")?;

    Ok(())
}

/// Updates conversation ordering metadata inside an existing transaction.
pub(super) fn touch_conversation_in_transaction(
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
pub(super) fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
mod tests;
