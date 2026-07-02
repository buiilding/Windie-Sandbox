use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::conversation::Message;

const DEFAULT_CONVERSATION_ID: &str = "default";
const ACTIVE_CONVERSATION_KEY: &str = "active_conversation_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationInfo {
    pub id: String,
    pub title: Option<String>,
    pub message_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct Compaction {
    pub id: String,
    pub conversation_id: String,
    pub through_message_id: String,
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
        store.migrate()?;

        Ok(store)
    }

    #[cfg(test)]
    fn open_memory() -> Result<Self> {
        let store = Self {
            connection: Connection::open_in_memory().context("failed to open memory database")?,
        };
        store.migrate()?;

        Ok(store)
    }

    pub fn migrate(&self) -> Result<()> {
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

                CREATE TABLE IF NOT EXISTS app_state (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS messages_conversation_created_idx
                ON messages(conversation_id, created_at);

                CREATE INDEX IF NOT EXISTS compactions_conversation_created_idx
                ON compactions(conversation_id, created_at);
                ",
            )
            .context("failed to migrate database")
    }

    pub fn create_conversation(&self) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT INTO conversations (id, title, created_at, updated_at)
                VALUES (?1, NULL, ?2, ?2)
                ",
                params![id, now],
            )
            .context("failed to create conversation")?;

        Ok(id)
    }

    pub fn get_or_create_default_conversation(&self) -> Result<String> {
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

        Ok(DEFAULT_CONVERSATION_ID.to_string())
    }

    pub fn get_or_create_active_conversation(&self) -> Result<String> {
        if let Some(conversation_id) = self.active_conversation_id()? {
            if self.conversation_exists(&conversation_id)? {
                return Ok(conversation_id);
            }
        }

        let conversation_id = self.get_or_create_default_conversation()?;
        self.set_active_conversation(&conversation_id)?;

        Ok(conversation_id)
    }

    pub fn active_conversation_id(&self) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT value FROM app_state WHERE key = ?1",
                params![ACTIVE_CONVERSATION_KEY],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load active conversation")
    }

    pub fn set_active_conversation(&self, conversation_id: &str) -> Result<()> {
        if !self.conversation_exists(conversation_id)? {
            return Err(anyhow!("conversation does not exist: {conversation_id}"));
        }

        self.connection
            .execute(
                "
                INSERT INTO app_state (key, value)
                VALUES (?1, ?2)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                ",
                params![ACTIVE_CONVERSATION_KEY, conversation_id],
            )
            .context("failed to set active conversation")?;

        Ok(())
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
                    id: row.get(0)?,
                    title: row.get(1)?,
                    message_count: row.get(2)?,
                })
            })
            .context("failed to list conversations")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read conversations")?;

        Ok(conversations)
    }

    pub fn load_messages(&self, conversation_id: &str) -> Result<Vec<Message>> {
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
            .query_map(params![conversation_id], |row| {
                Ok(Message {
                    id: Some(row.get(0)?),
                    parent_message_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                })
            })
            .context("failed to load messages")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages")?;

        Ok(messages)
    }

    #[allow(dead_code)]
    pub fn load_messages_after(
        &self,
        conversation_id: &str,
        message_id: Option<&str>,
    ) -> Result<Vec<Message>> {
        let Some(message_id) = message_id else {
            return self.load_messages(conversation_id);
        };

        let (created_at, rowid) = self
            .message_position(message_id)?
            .ok_or_else(|| anyhow!("message does not exist: {message_id}"))?;

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
            .query_map(params![conversation_id, created_at, rowid], |row| {
                Ok(Message {
                    id: Some(row.get(0)?),
                    parent_message_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                })
            })
            .context("failed to load messages after checkpoint")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages after checkpoint")?;

        Ok(messages)
    }

    pub fn save_message(
        &self,
        conversation_id: &str,
        parent_message_id: Option<&str>,
        role: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = now_millis()?;

        self.connection
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
                    id,
                    conversation_id,
                    parent_message_id,
                    role,
                    content,
                    metadata,
                    now
                ],
            )
            .context("failed to save message")?;

        self.touch_conversation(conversation_id, now)?;

        Ok(id)
    }

    #[allow(dead_code)]
    pub fn latest_compaction(&self, conversation_id: &str) -> Result<Option<Compaction>> {
        self.connection
            .query_row(
                "
                SELECT id, conversation_id, through_message_id, content, created_at
                FROM compactions
                WHERE conversation_id = ?1
                ORDER BY created_at DESC, rowid DESC
                LIMIT 1
                ",
                params![conversation_id],
                |row| {
                    Ok(Compaction {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        through_message_id: row.get(2)?,
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
        &self,
        conversation_id: &str,
        through_message_id: &str,
        content: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = now_millis()?;

        self.connection
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
                params![id, conversation_id, through_message_id, content, now],
            )
            .context("failed to save compaction")?;

        self.touch_conversation(conversation_id, now)?;

        Ok(id)
    }

    #[allow(dead_code)]
    fn message_position(&self, message_id: &str) -> Result<Option<(i64, i64)>> {
        self.connection
            .query_row(
                "SELECT created_at, rowid FROM messages WHERE id = ?1",
                params![message_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("failed to load message position")
    }

    fn touch_conversation(&self, conversation_id: &str, updated_at: i64) -> Result<()> {
        self.connection
            .execute(
                "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                params![updated_at, conversation_id],
            )
            .context("failed to update conversation timestamp")?;

        Ok(())
    }

    fn conversation_exists(&self, conversation_id: &str) -> Result<bool> {
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM conversations WHERE id = ?1",
                params![conversation_id],
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

fn now_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_default_conversation() {
        let store = Store::open_memory().unwrap();

        let conversation_id = store.get_or_create_default_conversation().unwrap();

        assert_eq!(conversation_id, "default");
    }

    #[test]
    fn creates_conversation_with_unique_id() {
        let store = Store::open_memory().unwrap();

        let first_id = store.create_conversation().unwrap();
        let second_id = store.create_conversation().unwrap();

        assert_ne!(first_id, second_id);
    }

    #[test]
    fn tracks_active_conversation() {
        let store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation().unwrap();

        store.set_active_conversation(&conversation_id).unwrap();

        assert_eq!(
            store.active_conversation_id().unwrap().as_deref(),
            Some(conversation_id.as_str())
        );
    }

    #[test]
    fn rejects_missing_active_conversation() {
        let store = Store::open_memory().unwrap();

        let error = store.set_active_conversation("missing").unwrap_err();

        assert!(error.to_string().contains("conversation does not exist"));
    }

    #[test]
    fn creates_active_default_conversation_when_none_is_active() {
        let store = Store::open_memory().unwrap();

        let conversation_id = store.get_or_create_active_conversation().unwrap();

        assert_eq!(conversation_id, "default");
        assert_eq!(
            store.active_conversation_id().unwrap().as_deref(),
            Some("default")
        );
    }

    #[test]
    fn lists_conversations() {
        let store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation().unwrap();
        store
            .save_message(&conversation_id, None, "user", "hello", None)
            .unwrap();

        let conversations = store.list_conversations().unwrap();

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].id, conversation_id);
        assert_eq!(conversations[0].message_count, 1);
    }

    #[test]
    fn saves_and_loads_messages() {
        let store = Store::open_memory().unwrap();
        let conversation_id = store.get_or_create_default_conversation().unwrap();

        let user_id = store
            .save_message(&conversation_id, None, "user", "hello", None)
            .unwrap();
        let assistant_id = store
            .save_message(
                &conversation_id,
                Some(&user_id),
                "assistant",
                "hello back",
                None,
            )
            .unwrap();

        let messages = store.load_messages(&conversation_id).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id.as_deref(), Some(user_id.as_str()));
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].id.as_deref(), Some(assistant_id.as_str()));
        assert_eq!(
            messages[1].parent_message_id.as_deref(),
            Some(user_id.as_str())
        );
        assert_eq!(messages[1].content, "hello back");
    }

    #[test]
    fn preserves_metadata() {
        let store = Store::open_memory().unwrap();
        let conversation_id = store.get_or_create_default_conversation().unwrap();

        store
            .save_message(
                &conversation_id,
                None,
                "assistant",
                "",
                Some(r#"{"tool_calls":[]}"#),
            )
            .unwrap();

        let messages = store.load_messages(&conversation_id).unwrap();

        assert_eq!(
            messages[0].metadata.as_deref(),
            Some(r#"{"tool_calls":[]}"#)
        );
    }

    #[test]
    fn loads_messages_after_checkpoint() {
        let store = Store::open_memory().unwrap();
        let conversation_id = store.get_or_create_default_conversation().unwrap();

        let first_id = store
            .save_message(&conversation_id, None, "user", "one", None)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second_id = store
            .save_message(&conversation_id, Some(&first_id), "assistant", "two", None)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store
            .save_message(&conversation_id, Some(&second_id), "user", "three", None)
            .unwrap();

        let messages = store
            .load_messages_after(&conversation_id, Some(&first_id))
            .unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "two");
        assert_eq!(messages[1].content, "three");
    }

    #[test]
    fn saves_and_loads_latest_compaction() {
        let store = Store::open_memory().unwrap();
        let conversation_id = store.get_or_create_default_conversation().unwrap();
        let message_id = store
            .save_message(&conversation_id, None, "user", "hello", None)
            .unwrap();

        let compaction_id = store
            .save_compaction(&conversation_id, &message_id, "summary")
            .unwrap();

        let compaction = store.latest_compaction(&conversation_id).unwrap().unwrap();

        assert_eq!(compaction.id, compaction_id);
        assert_eq!(compaction.conversation_id, conversation_id);
        assert_eq!(compaction.through_message_id, message_id);
        assert_eq!(compaction.content, "summary");
    }
}
