//! SQLite persistence boundary.
//!
//! This module owns persisted conversations, messages, and compactions. Other
//! modules should not know about SQLite tables or queries.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use uuid::Uuid;

use crate::conversation::{CompactionId, ConversationId, Message, MessageId, Role};

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
const DATABASE_SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationInfo {
    pub id: ConversationId,
    pub title: Option<String>,
    pub message_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compaction {
    pub id: CompactionId,
    pub conversation_id: ConversationId,
    pub through_message_id: MessageId,
    pub content: String,
    pub created_at: i64,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::open_at(default_database_path()?)
    }

    pub fn open_at(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent).context("failed to create database directory")?;
        }

        let store = Self {
            connection: Connection::open(path).context("failed to open database")?,
        };
        store.configure()?;
        store.migrate()?;

        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn open_memory() -> Result<Self> {
        let store = Self {
            connection: Connection::open_in_memory().context("failed to open memory database")?,
        };
        store.configure()?;
        store.migrate()?;

        Ok(store)
    }

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

                CREATE INDEX IF NOT EXISTS compactions_conversation_created_idx
                ON compactions(conversation_id, created_at);
                ",
            )
            .context("failed to migrate database")?;

        self.connection
            .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION)
            .context("failed to set database schema version")
    }

    fn database_schema_version(&self) -> Result<i32> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("failed to read database schema version")
    }

    pub fn create_conversation(&self) -> Result<ConversationId> {
        let id = ConversationId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT INTO conversations (id, title, created_at, updated_at)
                VALUES (?1, NULL, ?2, ?2)
                ",
                params![id.as_str(), now],
            )
            .context("failed to create conversation")?;

        Ok(id)
    }

    #[cfg(test)]
    pub(crate) fn get_or_create_default_conversation(&self) -> Result<ConversationId> {
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT OR IGNORE INTO conversations (id, title, created_at, updated_at)
                VALUES (?1, NULL, ?2, ?2)
                ",
                params![DEFAULT_CONVERSATION_ID, now],
            )
            .context("failed to create default conversation")?;

        Ok(ConversationId::new(DEFAULT_CONVERSATION_ID))
    }

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

    pub fn append_message(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        role: Role,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<MessageId> {
        let id = MessageId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;

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
                    metadata,
                    now
                ],
            )
            .context("failed to save message")?;

        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message save")?;

        Ok(id)
    }

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
        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start message delete transaction")?;

        delete_compactions_for_conversation(&transaction, conversation_id)
            .context("failed to delete compactions after message delete")?;
        transaction
            .execute(
                "
                UPDATE messages
                SET parent_message_id = ?1
                WHERE conversation_id = ?2 AND parent_message_id = ?3
                ",
                params![
                    parent_message_id.as_deref(),
                    conversation_id.as_str(),
                    message_id.as_str()
                ],
            )
            .context("failed to reconnect child messages")?;
        transaction
            .execute(
                "DELETE FROM messages WHERE conversation_id = ?1 AND id = ?2",
                params![conversation_id.as_str(), message_id.as_str()],
            )
            .context("failed to delete message")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit message delete")?;

        Ok(())
    }

    pub fn truncate_after_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let (created_at, rowid) = self
            .message_position(conversation_id, message_id)?
            .ok_or_else(|| anyhow!("message does not exist in conversation: {message_id}"))?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation truncate transaction")?;

        delete_compactions_for_conversation(&transaction, conversation_id)
            .context("failed to delete compactions after conversation truncate")?;
        transaction
            .execute(
                "
                DELETE FROM messages
                WHERE conversation_id = ?1
                  AND (
                    created_at > ?2
                    OR (created_at = ?2 AND rowid > ?3)
                  )
                ",
                params![conversation_id.as_str(), created_at, rowid],
            )
            .context("failed to truncate conversation messages")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit conversation truncate")?;

        Ok(())
    }

    pub fn fork_conversation_at_message(
        &mut self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<ConversationId> {
        self.ensure_conversation_exists(conversation_id)?;
        let (created_at, rowid) = self
            .message_position(conversation_id, message_id)?
            .ok_or_else(|| anyhow!("message does not exist in conversation: {message_id}"))?;

        let source_messages = self
            .load_messages_through_position(conversation_id, created_at, rowid)
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
                INSERT INTO conversations (id, title, created_at, updated_at)
                VALUES (?1, NULL, ?2, ?2)
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
                        message.metadata.as_deref(),
                        now + index as i64
                    ],
                )
                .context("failed to copy forked conversation message")?;

            message_id_map.insert(source_message_id.as_str().to_string(), forked_message_id);
        }

        transaction
            .commit()
            .context("failed to commit conversation fork")?;

        Ok(forked_conversation_id)
    }

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

    fn load_messages_through_position(
        &self,
        conversation_id: &ConversationId,
        created_at: i64,
        rowid: i64,
    ) -> Result<Vec<Message>> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_message_id, role, content, metadata
                FROM messages
                WHERE conversation_id = ?1
                  AND (
                    created_at < ?2
                    OR (created_at = ?2 AND rowid <= ?3)
                  )
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare message load through checkpoint")?;

        let messages = statement
            .query_map(params![conversation_id.as_str(), created_at, rowid], read_message_row)
            .context("failed to load messages through checkpoint")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages through checkpoint")?;

        Ok(messages)
    }

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

    fn ensure_conversation_exists(&self, conversation_id: &ConversationId) -> Result<()> {
        if !self.conversation_exists(conversation_id)? {
            return Err(anyhow!("conversation does not exist: {conversation_id}"));
        }

        Ok(())
    }

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

fn default_database_path() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;

    Ok(PathBuf::from(home).join(".windie").join("windie.db"))
}

fn read_message_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: Some(MessageId::new(row.get::<_, String>(0)?)),
        parent_message_id: row.get::<_, Option<String>>(1)?.map(MessageId::new),
        role: row.get(2)?,
        content: row.get(3)?,
        metadata: row.get(4)?,
    })
}

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

fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
