//! SQLite schema creation and version validation.

use super::*;

pub(super) const DATABASE_SCHEMA_VERSION: i32 = 16;

impl Store {
    /// Creates or validates the current schema.
    ///
    /// Windie refuses to open databases from any other schema version. This
    /// keeps the current foundation clean while schema compatibility is not a
    /// supported project goal.
    pub fn migrate(&self) -> Result<()> {
        let existing_version = self.database_schema_version()?;
        if existing_version > DATABASE_SCHEMA_VERSION {
            return Err(anyhow!(
                "database schema version {existing_version} is newer than supported version {DATABASE_SCHEMA_VERSION}"
            ));
        }
        if existing_version != 0 && existing_version < DATABASE_SCHEMA_VERSION {
            return Err(anyhow!(
                "database schema version {existing_version} is older than supported version {DATABASE_SCHEMA_VERSION}; remove the old Windie database or recreate it"
            ));
        }
        if existing_version == 0 && self.table_exists("conversations")? {
            return Err(anyhow!(
                "existing unversioned Windie database is not supported; remove the old Windie database or recreate it"
            ));
        }

        self.connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS conversations (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    model TEXT NOT NULL,
                    reasoning_effort TEXT,
                    tool_approval_mode TEXT NOT NULL,
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

                CREATE TABLE IF NOT EXISTS image_assets (
                    id TEXT PRIMARY KEY,
                    bytes BLOB NOT NULL,
                    mime_type TEXT NOT NULL,
                    sha256 TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS message_parts (
                    id TEXT PRIMARY KEY,
                    message_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    text TEXT,
                    image_asset_id TEXT,

                    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE,
                    FOREIGN KEY (image_asset_id) REFERENCES image_assets(id)
                );

                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    start_head_message_id TEXT,
                    current_head_message_id TEXT,
                    status TEXT NOT NULL,
                    model TEXT NOT NULL,
                    reasoning TEXT,
                    error TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,

                    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
                    FOREIGN KEY (start_head_message_id) REFERENCES messages(id),
                    FOREIGN KEY (current_head_message_id) REFERENCES messages(id)
                );

                CREATE TABLE IF NOT EXISTS session_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    event_type TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    created_at INTEGER NOT NULL,

                    FOREIGN KEY (session_id) REFERENCES sessions(id)
                );

                CREATE INDEX IF NOT EXISTS idx_sessions_conversation
                ON sessions(conversation_id);

                CREATE INDEX IF NOT EXISTS idx_session_events_run_id_id
                ON session_events(session_id, id);

                CREATE TABLE IF NOT EXISTS compactions (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    through_message_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at INTEGER NOT NULL,

                    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
                    FOREIGN KEY (through_message_id) REFERENCES messages(id)
                );

                CREATE TABLE IF NOT EXISTS tool_schemas (
                    conversation_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    parameters_json TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    provider_tool_name TEXT NOT NULL,
                    provider_kind TEXT NOT NULL,
                    permissions_json TEXT NOT NULL,
                    annotations_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,

                    PRIMARY KEY (conversation_id, name),
                    FOREIGN KEY (conversation_id) REFERENCES conversations(id)
                );

                CREATE INDEX IF NOT EXISTS messages_conversation_created_idx
                ON messages(conversation_id, created_at);

                CREATE INDEX IF NOT EXISTS messages_id_conversation_idx
                ON messages(id, conversation_id);

                CREATE INDEX IF NOT EXISTS messages_parent_idx
                ON messages(conversation_id, parent_message_id);

                CREATE INDEX IF NOT EXISTS message_parts_message_idx
                ON message_parts(message_id, position);

                CREATE INDEX IF NOT EXISTS conversations_updated_idx
                ON conversations(updated_at);

                CREATE INDEX IF NOT EXISTS compactions_conversation_created_idx
                ON compactions(conversation_id, created_at);

                CREATE INDEX IF NOT EXISTS tool_schemas_conversation_created_idx
                ON tool_schemas(conversation_id, created_at);
                ",
            )
            .context("failed to migrate database")?;

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
