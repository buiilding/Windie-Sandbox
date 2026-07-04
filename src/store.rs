//! SQLite persistence boundary.
//!
//! This module owns persisted conversations, messages, and compactions. Other
//! modules should not know about SQLite tables or queries.

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
    MessagePart, Role,
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
const DATABASE_SCHEMA_VERSION: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Lightweight row used by conversation listing.
pub struct ConversationInfo {
    pub id: ConversationId,
    pub title: Option<String>,
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

    /// Creates or validates the current schema.
    ///
    /// Windie refuses to open databases from a newer schema version because this
    /// binary may not understand their shape.
    pub fn migrate(&self) -> Result<()> {
        let existing_version = self.database_schema_version()?;
        if existing_version > DATABASE_SCHEMA_VERSION {
            return Err(anyhow!(
                "database schema version {existing_version} is newer than supported version {DATABASE_SCHEMA_VERSION}"
            ));
        }

        self.connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS conversations (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    active_message_id TEXT,
                    system_prompt TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS messages (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    parent_message_id TEXT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    metadata TEXT,
                    created_at INTEGER NOT NULL,

                    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
                    FOREIGN KEY (parent_message_id) REFERENCES messages(id)
                );

                CREATE TABLE IF NOT EXISTS compactions (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    through_message_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at INTEGER NOT NULL,

                    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
                    FOREIGN KEY (through_message_id) REFERENCES messages(id)
                );

                CREATE INDEX IF NOT EXISTS messages_conversation_created_idx
                ON messages(conversation_id, created_at);

                CREATE INDEX IF NOT EXISTS messages_parent_idx
                ON messages(conversation_id, parent_message_id);

                CREATE INDEX IF NOT EXISTS conversations_updated_idx
                ON conversations(updated_at);

                CREATE INDEX IF NOT EXISTS compactions_conversation_created_idx
                ON compactions(conversation_id, created_at);
                ",
            )
            .context("failed to migrate database")?;

        if !self
            .conversation_has_column("active_message_id")
            .context("failed to inspect conversation columns")?
        {
            self.connection
                .execute(
                    "ALTER TABLE conversations ADD COLUMN active_message_id TEXT",
                    [],
                )
                .context("failed to add active message column")?;
        }

        if !self
            .conversation_has_column("system_prompt")
            .context("failed to inspect conversation columns")?
        {
            self.connection
                .execute(
                    "ALTER TABLE conversations ADD COLUMN system_prompt TEXT",
                    [],
                )
                .context("failed to add system prompt column")?;
        }

        self.backfill_active_messages()
            .context("failed to backfill active messages")?;

        self.connection
            .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION)
            .context("failed to set database schema version")
    }

    /// Reads SQLite's schema version marker.
    fn database_schema_version(&self) -> Result<i32> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("failed to read database schema version")
    }

    /// Creates an empty conversation with a generated ID.
    pub fn create_conversation(&self) -> Result<ConversationId> {
        let id = ConversationId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT INTO conversations (id, title, active_message_id, created_at, updated_at)
                VALUES (?1, NULL, NULL, ?2, ?2)
                ",
                params![id.as_str(), now],
            )
            .context("failed to create conversation")?;

        Ok(id)
    }

    #[cfg(test)]
    /// Creates a deterministic conversation ID for tests that need predictable
    /// setup.
    pub(crate) fn get_or_create_default_conversation(&self) -> Result<ConversationId> {
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT OR IGNORE INTO conversations (id, title, active_message_id, created_at, updated_at)
                VALUES (?1, NULL, NULL, ?2, ?2)
                ",
                params![DEFAULT_CONVERSATION_ID, now],
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

        let messages = statement
            .query_map(params![conversation_id.as_str()], read_message_row)
            .context("failed to load messages")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages")?;

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
                    FROM messages
                    JOIN path ON messages.id = path.parent_message_id
                    WHERE messages.conversation_id = ?1
                )
                SELECT id, parent_message_id, role, content, metadata
                FROM path
                ORDER BY depth DESC
                ",
            )
            .context("failed to prepare active path load")?;

        let path = statement
            .query_map(
                params![conversation_id.as_str(), message_id.as_str()],
                read_message_row,
            )
            .context("failed to load active path")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read active path")?;

        Ok(path)
    }

    /// Loads messages after an optional checkpoint message in insertion order.
    ///
    /// This is intentionally not part of the active query path. It is kept as a
    /// future compaction/checkpoint primitive for code that needs chronological
    /// suffixes rather than root-to-active tree paths.
    #[allow(dead_code)]
    pub fn load_messages_after(
        &self,
        conversation_id: &ConversationId,
        message_id: Option<&MessageId>,
    ) -> Result<Vec<Message>> {
        self.ensure_conversation_exists(conversation_id)?;

        let Some(message_id) = message_id else {
            return self.load_messages(conversation_id);
        };

        let (created_at, rowid) = self
            .message_position(conversation_id, message_id)?
            .ok_or_else(|| anyhow!("message does not exist in conversation: {message_id}"))?;

        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_message_id, role, content, metadata
                FROM messages
                WHERE conversation_id = ?1
                  AND (
                    created_at > ?2
                    OR (created_at = ?2 AND rowid > ?3)
                  )
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare message load after checkpoint")?;

        let messages = statement
            .query_map(
                params![conversation_id.as_str(), created_at, rowid],
                read_message_row,
            )
            .context("failed to load messages after checkpoint")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages after checkpoint")?;

        Ok(messages)
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
        delete_compactions_for_conversation(&transaction, conversation_id)
            .context("failed to delete compactions after message update")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message update")?;

        Ok(())
    }

    /// Deletes one message and all descendants below it.
    ///
    /// If the active message is inside the deleted subtree, active moves to the
    /// deleted message's parent.
    pub fn remove_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;

        let parent_message_id = self
            .connection
            .query_row(
                "
                SELECT parent_message_id
                FROM messages
                WHERE conversation_id = ?1 AND id = ?2
                ",
                params![conversation_id.as_str(), message_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .context("failed to load message parent")?;
        let subtree_ids = self
            .descendant_message_ids(conversation_id, message_id, true)
            .context("failed to load message subtree")?;
        let active_message_id = self.active_message_id(conversation_id)?;
        let next_active_message_id = if active_message_id
            .as_ref()
            .is_some_and(|active_message_id| subtree_ids.contains(active_message_id.as_str()))
        {
            parent_message_id.as_deref().map(MessageId::new)
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
        transaction
            .execute(
                "
                WITH RECURSIVE subtree(id) AS (
                    SELECT ?2
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
            .context("failed to delete message subtree")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message delete")?;

        Ok(())
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
                INSERT INTO conversations (id, title, active_message_id, created_at, updated_at)
                VALUES (?1, NULL, NULL, ?2, ?2)
                ",
                params![forked_conversation_id.as_str(), now],
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
                "DELETE FROM messages WHERE conversation_id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation messages")?;
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

    /// Returns insertion position for a message inside a conversation.
    ///
    /// This helper exists only for chronological suffix loading used by
    /// compaction/checkpoint work. `rowid` breaks ties when multiple rows share
    /// the same millisecond timestamp.
    #[allow(dead_code)]
    fn message_position(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<Option<(i64, i64)>> {
        self.connection
            .query_row(
                "
                SELECT created_at, rowid
                FROM messages
                WHERE conversation_id = ?1 AND id = ?2
                ",
                params![conversation_id.as_str(), message_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("failed to load message position")
    }

    /// Loads messages from the beginning through a chronological checkpoint.
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
            .ok_or_else(|| anyhow!("message does not exist: {message_id}"))?;

        if message_conversation_id != *conversation_id {
            return Err(anyhow!(
                "message does not belong to conversation: {message_id}"
            ));
        }

        Ok(())
    }

    /// Returns an error instead of silently treating missing conversations as
    /// empty.
    fn ensure_conversation_exists(&self, conversation_id: &ConversationId) -> Result<()> {
        if !self.conversation_exists(conversation_id)? {
            return Err(anyhow!("conversation does not exist: {conversation_id}"));
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

    fn conversation_has_column(&self, column_name: &str) -> Result<bool> {
        let mut statement = self
            .connection
            .prepare("PRAGMA table_info(conversations)")
            .context("failed to prepare conversation column inspection")?;
        let exists = statement
            .query_map([], |row| row.get::<_, String>(1))
            .context("failed to inspect conversation columns")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read conversation columns")?
            .iter()
            .any(|column| column == column_name);

        Ok(exists)
    }

    fn backfill_active_messages(&self) -> Result<()> {
        self.connection
            .execute(
                "
                UPDATE conversations
                SET active_message_id = (
                    SELECT messages.id
                    FROM messages
                    WHERE messages.conversation_id = conversations.id
                    ORDER BY messages.created_at DESC, messages.rowid DESC
                    LIMIT 1
                )
                WHERE active_message_id IS NULL
                  AND EXISTS (
                    SELECT 1
                    FROM messages
                    WHERE messages.conversation_id = conversations.id
                  )
                ",
                [],
            )
            .context("failed to backfill active messages")?;

        Ok(())
    }
}

/// Builds the default user database path.
fn default_database_path() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;

    Ok(PathBuf::from(home).join(".windie").join("windie.db"))
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

/// Writes all ordered parts for one message into an existing transaction.
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

/// Copies image bytes into image asset storage inside an existing transaction.
fn insert_image_asset_in_transaction(
    transaction: &Transaction<'_>,
    mime_type: &str,
    bytes: &[u8],
    now: i64,
) -> Result<ImageAssetId> {
    let asset_id = ImageAssetId::new(Uuid::new_v4().to_string());
    let sha256 = sha256_hex(bytes);

    transaction
        .execute(
            "
            INSERT INTO image_assets (id, bytes, mime_type, sha256, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ",
            params![asset_id.as_str(), bytes, mime_type, sha256, now],
        )
        .context("failed to save image asset")?;

    Ok(asset_id)
}

/// Links one image asset to an ordered message part.
fn insert_image_part_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    position: usize,
    image_asset_id: &ImageAssetId,
) -> Result<()> {
    transaction
        .execute(
            "
            INSERT INTO message_parts (id, message_id, position, kind, text, image_asset_id)
            VALUES (?1, ?2, ?3, 'image', NULL, ?4)
            ",
            params![
                Uuid::new_v4().to_string(),
                message_id.as_str(),
                position as i64,
                image_asset_id.as_str()
            ],
        )
        .context("failed to save image message part")?;

    Ok(())
}

/// Removes image assets no remaining message part references.
fn delete_orphan_image_assets_in_transaction(transaction: &Transaction<'_>) -> Result<()> {
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

/// Returns lowercase hex SHA-256 text for stored asset identity metadata.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
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

/// Returns current Unix time in milliseconds for ordering persisted rows.
fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
