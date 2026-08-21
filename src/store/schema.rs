//! SQLite schema creation and version validation.

use super::*;

pub(super) const DATABASE_SCHEMA_VERSION: i32 = 25;
const PREVIOUS_DATABASE_SCHEMA_VERSION: i32 = 24;

impl Store {
    /// Creates or validates the current schema.
    ///
    /// The runtime-access table is an additive security migration from schema
    /// version 24. Other unsupported historical versions still fail closed
    /// rather than guessing how to transform durable runtime data.
    pub fn migrate(&self) -> Result<()> {
        let existing_version = self.database_schema_version()?;
        if existing_version > DATABASE_SCHEMA_VERSION {
            return Err(anyhow!(
                "database schema version {existing_version} is newer than supported version {DATABASE_SCHEMA_VERSION}"
            ));
        }
        if existing_version != 0 && existing_version < PREVIOUS_DATABASE_SCHEMA_VERSION {
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
                    execution_owner TEXT,
                    execution_claim_id TEXT,
                    keep_awake INTEGER NOT NULL DEFAULT 0,
                    last_user_activity_at INTEGER NOT NULL,
                    last_idle_wakeup_completed_at INTEGER,
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

                CREATE TABLE IF NOT EXISTS session_inputs (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    parts_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL,

                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_sessions_conversation
                ON sessions(conversation_id);

                CREATE INDEX IF NOT EXISTS idx_session_events_run_id_id
                ON session_events(session_id, id);

                CREATE INDEX IF NOT EXISTS idx_session_inputs_session_created
                ON session_inputs(session_id, created_at);

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

                CREATE TABLE IF NOT EXISTS installed_providers (
                    provider_id TEXT PRIMARY KEY,
                    state TEXT NOT NULL,
                    readiness TEXT NOT NULL,
                    next_action TEXT,
                    error TEXT,
                    installed_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    last_health_check_at INTEGER
                );

                CREATE TABLE IF NOT EXISTS chrome_devtools_settings (
                    provider_id TEXT PRIMARY KEY,
                    mode TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,

                    FOREIGN KEY (provider_id) REFERENCES installed_providers(provider_id)
                        ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS provider_tool_catalogs (
                    provider_id TEXT PRIMARY KEY,
                    tools_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    discovered_at INTEGER,
                    last_error TEXT,

                    FOREIGN KEY (provider_id) REFERENCES installed_providers(provider_id)
                        ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS runtime_access (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    account_id TEXT NOT NULL,
                    linked_at INTEGER NOT NULL
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

                CREATE INDEX IF NOT EXISTS installed_providers_updated_idx
                ON installed_providers(updated_at);

                CREATE INDEX IF NOT EXISTS provider_tool_catalogs_status_idx
                ON provider_tool_catalogs(status);
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
