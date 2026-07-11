//! Database opening, configuration, and schema creation.

use super::*;

impl Store {
    /// Opens the default user database in Windie's data directory.
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
                    model TEXT NOT NULL,
                    reasoning_effort TEXT,
                    active_message_id TEXT,
                    system_prompt TEXT,
                    tool_approval_mode TEXT NOT NULL,
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
                    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS runtime_runs (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    action TEXT NOT NULL,
                    owner_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    error TEXT,
                    lease_expires_at INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,

                    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS runtime_run_events (
                    run_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL,
                    payload_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL,

                    PRIMARY KEY (run_id, sequence),
                    FOREIGN KEY (run_id) REFERENCES runtime_runs(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS tool_call_executions (
                    conversation_id TEXT NOT NULL,
                    assistant_message_id TEXT NOT NULL,
                    tool_call_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    result_message_id TEXT,
                    error TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,

                    PRIMARY KEY (assistant_message_id, tool_call_id),
                    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
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

                CREATE INDEX IF NOT EXISTS runtime_runs_conversation_updated_idx
                ON runtime_runs(conversation_id, updated_at);

                CREATE UNIQUE INDEX IF NOT EXISTS runtime_runs_one_running_per_conversation_idx
                ON runtime_runs(conversation_id)
                WHERE status = 'running';

                CREATE INDEX IF NOT EXISTS runtime_run_events_run_sequence_idx
                ON runtime_run_events(run_id, sequence);
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
}

impl Store {
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
fn default_database_path() -> Result<PathBuf> {
    Ok(paths::database_path())
}
