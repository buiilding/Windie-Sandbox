//! SQLite persistence boundary.
//!
//! This module owns persisted conversations, messages, compactions, and
//! attached tools. Other modules should not know about SQLite tables or
//! queries.

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
    MessagePart, Role, ToolCallId, ToolSchema, ToolSchemaName, UnsavedMessagePart,
};
use crate::error;
use crate::llm::ReasoningRequest;
use crate::run::{Run, RunEvent, RunEventRecord, RunId, RunStatus};
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolApprovalMode, ToolPermission,
    ToolProviderId, ToolProviderKind, ToolProviderRef,
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
const DATABASE_SCHEMA_VERSION: i32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Lightweight row used by conversation listing.
pub struct ConversationInfo {
    pub id: ConversationId,
    pub title: Option<String>,
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

                CREATE TABLE IF NOT EXISTS runtime_runs (
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

                CREATE TABLE IF NOT EXISTS runtime_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id TEXT NOT NULL,
                    event_type TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    created_at INTEGER NOT NULL,

                    FOREIGN KEY (run_id) REFERENCES runtime_runs(id)
                );

                CREATE INDEX IF NOT EXISTS idx_runtime_runs_conversation
                ON runtime_runs(conversation_id);

                CREATE INDEX IF NOT EXISTS idx_runtime_events_run_id_id
                ON runtime_events(run_id, id);

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

    /// Creates an empty conversation with a generated ID and persisted model.
    pub fn create_conversation(&self, model: &str) -> Result<ConversationId> {
        let model = normalize_conversation_model(model)?;
        let id = ConversationId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT INTO conversations (
                    id,
                    title,
                    model,
                    reasoning_effort,
                    active_message_id,
                    system_prompt,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, NULL, ?2, NULL, NULL, NULL, ?3, ?4, ?4)
                ",
                params![
                    id.as_str(),
                    model,
                    ToolApprovalMode::Manual.as_storage(),
                    now
                ],
            )
            .context("failed to create conversation")?;

        Ok(id)
    }

    #[cfg(test)]
    /// Creates a deterministic conversation ID for tests that need predictable
    /// setup.
    pub(crate) fn get_or_create_default_conversation(&self, model: &str) -> Result<ConversationId> {
        let model = normalize_conversation_model(model)?;
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT OR IGNORE INTO conversations (
                    id,
                    title,
                    model,
                    reasoning_effort,
                    active_message_id,
                    system_prompt,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, NULL, ?2, NULL, NULL, NULL, ?3, ?4, ?4)
                ",
                params![
                    DEFAULT_CONVERSATION_ID,
                    model,
                    ToolApprovalMode::Manual.as_storage(),
                    now
                ],
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
                    conversations.model,
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
                    model: row.get(2)?,
                    message_count: row.get(3)?,
                })
            })
            .context("failed to list conversations")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read conversations")?;

        Ok(conversations)
    }

    /// Loads all messages for one conversation in stable insertion order.
    pub fn load_messages(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let mut messages = self.load_message_rows(conversation_id)?;
        self.attach_message_parts(&mut messages)
            .context("failed to load message parts")?;

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

    /// Loads the conversation's persisted default model.
    pub fn conversation_model(&self, conversation_id: &ConversationId) -> Result<String> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "SELECT model FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get(0),
            )
            .context("failed to load conversation model")
    }

    /// Loads the conversation-level reasoning effort for future queries.
    ///
    /// The store persists only the user/client-selected effort string. Provider
    /// request shaping, such as adding OpenAI's visible reasoning-summary flag,
    /// stays in the operation/LLM boundary where the concrete model is known.
    pub fn conversation_reasoning_effort(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<String>> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "SELECT reasoning_effort FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load conversation reasoning effort")
            .map(Option::flatten)
    }

    /// Sets the conversation's persisted default model.
    pub fn set_conversation_model(
        &mut self,
        conversation_id: &ConversationId,
        model: &str,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let model = normalize_conversation_model(model)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation model transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET model = ?1, reasoning_effort = NULL WHERE id = ?2",
                params![model, conversation_id.as_str()],
            )
            .context("failed to save conversation model")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit conversation model update")?;

        Ok(())
    }

    /// Sets the conversation-level reasoning effort used by future queries.
    ///
    /// `None` and blank strings clear the setting. The store intentionally does
    /// not validate model-specific values because Bifrost model metadata is the
    /// source of truth for which efforts are available for a selected model.
    pub fn set_conversation_reasoning_effort(
        &mut self,
        conversation_id: &ConversationId,
        effort: Option<&str>,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let effort = normalize_conversation_reasoning_effort(effort);

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start conversation reasoning transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET reasoning_effort = ?1 WHERE id = ?2",
                params![effort, conversation_id.as_str()],
            )
            .context("failed to save conversation reasoning effort")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit conversation reasoning update")?;

        Ok(())
    }

    /// Loads the conversation default for tool-call approval.
    pub fn tool_approval_mode(&self, conversation_id: &ConversationId) -> Result<ToolApprovalMode> {
        self.ensure_conversation_exists(conversation_id)?;

        let value = self
            .connection
            .query_row(
                "SELECT tool_approval_mode FROM conversations WHERE id = ?1",
                params![conversation_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .context("failed to load tool approval mode")?;

        ToolApprovalMode::from_storage(&value)
            .ok_or_else(|| anyhow!("unknown tool approval mode: {value}"))
    }

    /// Sets the conversation default for future tool-call approvals.
    pub fn set_tool_approval_mode(
        &mut self,
        conversation_id: &ConversationId,
        mode: ToolApprovalMode,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start tool approval mode transaction")?;

        transaction
            .execute(
                "UPDATE conversations SET tool_approval_mode = ?1 WHERE id = ?2",
                params![mode.as_storage(), conversation_id.as_str()],
            )
            .context("failed to save tool approval mode")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit tool approval mode update")?;

        Ok(())
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

    /// Clears the conversation-level system prompt without changing messages.
    pub fn remove_system_prompt(&mut self, conversation_id: &ConversationId) -> Result<()> {
        self.set_system_prompt(conversation_id, "")
    }

    /// Loads all attached provider tools configured on one conversation.
    ///
    /// Attached tools are conversation-level model inputs plus provider
    /// dispatch metadata. They are not message nodes and do not imply automatic
    /// execution; runtime still requires approval before provider calls.
    pub fn load_attached_tools(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<AttachedTool>> {
        self.ensure_conversation_exists(conversation_id)?;

        let mut statement = self
            .connection
            .prepare(
                "
                SELECT
                    name,
                    description,
                    parameters_json,
                    provider_id,
                    provider_tool_name,
                    provider_kind,
                    permissions_json,
                    annotations_json
                FROM tool_schemas
                WHERE conversation_id = ?1
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare attached tool load")?;

        let attached_tools = statement
            .query_map(params![conversation_id.as_str()], read_attached_tool_row)
            .context("failed to load attached tools")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read attached tools")?;

        Ok(attached_tools)
    }

    /// Loads the model-facing schema subset for attached tools.
    pub fn load_tool_schemas(&self, conversation_id: &ConversationId) -> Result<Vec<ToolSchema>> {
        Ok(self
            .load_attached_tools(conversation_id)?
            .into_iter()
            .map(|tool| tool.schema())
            .collect())
    }

    /// Loads one attached tool by its model-facing schema name.
    pub fn load_attached_tool(
        &self,
        conversation_id: &ConversationId,
        name: &ToolSchemaName,
    ) -> Result<Option<AttachedTool>> {
        self.ensure_conversation_exists(conversation_id)?;

        let attached_tool = self
            .connection
            .query_row(
                "
                SELECT
                    name,
                    description,
                    parameters_json,
                    provider_id,
                    provider_tool_name,
                    provider_kind,
                    permissions_json,
                    annotations_json
                FROM tool_schemas
                WHERE conversation_id = ?1 AND name = ?2
                ",
                params![conversation_id.as_str(), name.as_str()],
                read_attached_tool_row,
            )
            .optional()
            .context("failed to load attached tool")?;

        Ok(attached_tool)
    }

    /// Attaches one provider-backed tool to a conversation.
    pub fn insert_attached_tool(
        &mut self,
        conversation_id: &ConversationId,
        attached_tool: &AttachedTool,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        validate_attached_tool(attached_tool)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start attached tool insert transaction")?;

        insert_attached_tool_in_transaction(&transaction, conversation_id, attached_tool, now)
            .context("failed to attach tool")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit attached tool insert")?;

        Ok(())
    }

    /// Attaches multiple provider-backed tools as one atomic conversation
    /// mutation.
    pub fn insert_attached_tools(
        &mut self,
        conversation_id: &ConversationId,
        attached_tools: &[AttachedTool],
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        for attached_tool in attached_tools {
            validate_attached_tool(attached_tool)?;
        }
        if attached_tools.is_empty() {
            return Ok(());
        }

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start attached tools insert transaction")?;

        for attached_tool in attached_tools {
            insert_attached_tool_in_transaction(&transaction, conversation_id, attached_tool, now)
                .context("failed to attach tools")?;
        }
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit attached tools insert")?;

        Ok(())
    }

    /// Inserts one raw model-facing schema as a manual attached tool.
    pub fn insert_tool_schema(
        &mut self,
        conversation_id: &ConversationId,
        tool_schema: &ToolSchema,
    ) -> Result<()> {
        self.insert_attached_tool(conversation_id, &AttachedTool::manual(tool_schema.clone()))
    }

    /// Updates one existing tool schema, including an optional rename.
    pub fn update_tool_schema(
        &mut self,
        conversation_id: &ConversationId,
        current_name: &ToolSchemaName,
        tool_schema: &ToolSchema,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_tool_schema_exists(conversation_id, current_name)?;
        let attached_tool = AttachedTool::manual(tool_schema.clone());
        validate_attached_tool(&attached_tool)?;

        let now = now_millis()?;
        let parameters_json = encode_tool_parameters(&attached_tool.parameters)?;
        let permissions_json = encode_tool_permissions(&attached_tool.permissions)?;
        let annotations_json = encode_tool_annotations(&attached_tool.annotations)?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start attached tool update transaction")?;

        transaction
            .execute(
                "
                UPDATE tool_schemas
                SET name = ?1,
                    description = ?2,
                    parameters_json = ?3,
                    provider_id = ?4,
                    provider_tool_name = ?5,
                    provider_kind = ?6,
                    permissions_json = ?7,
                    annotations_json = ?8,
                    updated_at = ?9
                WHERE conversation_id = ?10 AND name = ?11
                ",
                params![
                    attached_tool.schema_name.as_str(),
                    attached_tool.description.as_str(),
                    parameters_json.as_str(),
                    attached_tool.provider.provider_id.as_str(),
                    attached_tool.provider.tool_name.as_str(),
                    attached_tool.provider.kind.as_storage(),
                    permissions_json.as_str(),
                    annotations_json.as_str(),
                    now,
                    conversation_id.as_str(),
                    current_name.as_str()
                ],
            )
            .context("failed to update attached tool")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit attached tool update")?;

        Ok(())
    }

    /// Removes one tool schema from a conversation.
    pub fn remove_tool_schema(
        &mut self,
        conversation_id: &ConversationId,
        name: &ToolSchemaName,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        self.ensure_tool_schema_exists(conversation_id, name)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start tool schema delete transaction")?;

        transaction
            .execute(
                "DELETE FROM tool_schemas WHERE conversation_id = ?1 AND name = ?2",
                params![conversation_id.as_str(), name.as_str()],
            )
            .context("failed to remove tool schema")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit tool schema delete")?;

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

    /// Creates one runtime run from an explicit conversation head.
    pub fn create_run(
        &mut self,
        run_id: &RunId,
        conversation_id: &ConversationId,
        start_head_message_id: Option<&MessageId>,
        model: &str,
        reasoning: Option<&ReasoningRequest>,
    ) -> Result<Run> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = start_head_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }

        let now = now_millis()?;
        let reasoning_json = reasoning
            .map(serde_json::to_string)
            .transpose()
            .context("failed to encode run reasoning")?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start runtime run transaction")?;

        transaction
            .execute(
                "
                INSERT INTO runtime_runs (
                    id,
                    conversation_id,
                    start_head_message_id,
                    current_head_message_id,
                    status,
                    model,
                    reasoning,
                    error,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8)
                ",
                params![
                    run_id.as_str(),
                    conversation_id.as_str(),
                    start_head_message_id.map(MessageId::as_str),
                    start_head_message_id.map(MessageId::as_str),
                    RunStatus::Running.as_storage(),
                    model,
                    reasoning_json.as_deref(),
                    now
                ],
            )
            .context("failed to create runtime run")?;
        transaction
            .commit()
            .context("failed to commit runtime run create")?;

        self.load_run(run_id)
    }

    /// Loads one runtime run by ID.
    pub fn load_run(&self, run_id: &RunId) -> Result<Run> {
        self.connection
            .query_row(
                "
                SELECT
                    id,
                    conversation_id,
                    start_head_message_id,
                    current_head_message_id,
                    status,
                    model,
                    reasoning,
                    error,
                    created_at,
                    updated_at
                FROM runtime_runs
                WHERE id = ?1
                ",
                params![run_id.as_str()],
                run_from_row,
            )
            .optional()
            .context("failed to load runtime run")?
            .ok_or_else(|| error::not_found(format!("runtime run does not exist: {run_id}")))
    }

    /// Lists all known runtime runs, newest first.
    pub fn list_runs(&self) -> Result<Vec<Run>> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT
                    id,
                    conversation_id,
                    start_head_message_id,
                    current_head_message_id,
                    status,
                    model,
                    reasoning,
                    error,
                    created_at,
                    updated_at
                FROM runtime_runs
                ORDER BY created_at DESC, id DESC
                ",
            )
            .context("failed to prepare runtime run list")?;

        statement
            .query_map([], run_from_row)
            .context("failed to list runtime runs")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode runtime runs")
    }

    /// Lists runs belonging to one conversation.
    pub fn list_conversation_runs(&self, conversation_id: &ConversationId) -> Result<Vec<Run>> {
        self.ensure_conversation_exists(conversation_id)?;

        let mut statement = self
            .connection
            .prepare(
                "
                SELECT
                    id,
                    conversation_id,
                    start_head_message_id,
                    current_head_message_id,
                    status,
                    model,
                    reasoning,
                    error,
                    created_at,
                    updated_at
                FROM runtime_runs
                WHERE conversation_id = ?1
                ORDER BY created_at DESC, id DESC
                ",
            )
            .context("failed to prepare conversation runtime run list")?;

        statement
            .query_map(params![conversation_id.as_str()], run_from_row)
            .context("failed to list conversation runtime runs")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode conversation runtime runs")
    }

    /// Updates one run's current message head.
    pub fn update_run_head(
        &mut self,
        run_id: &RunId,
        head_message_id: Option<&MessageId>,
    ) -> Result<()> {
        let run = self.load_run(run_id)?;
        if let Some(message_id) = head_message_id {
            self.ensure_message_belongs_to_conversation(&run.conversation_id, message_id)?;
        }

        let now = now_millis()?;
        self.connection
            .execute(
                "
                UPDATE runtime_runs
                SET current_head_message_id = ?1,
                    updated_at = ?2
                WHERE id = ?3
                ",
                params![head_message_id.map(MessageId::as_str), now, run_id.as_str()],
            )
            .context("failed to update runtime run head")?;

        Ok(())
    }

    /// Updates one run's lifecycle status.
    pub fn update_run_status(
        &mut self,
        run_id: &RunId,
        status: RunStatus,
        error: Option<&str>,
    ) -> Result<()> {
        self.ensure_run_exists(run_id)?;

        let now = now_millis()?;
        self.connection
            .execute(
                "
                UPDATE runtime_runs
                SET status = ?1,
                    error = ?2,
                    updated_at = ?3
                WHERE id = ?4
                ",
                params![status.as_storage(), error, now, run_id.as_str()],
            )
            .context("failed to update runtime run status")?;

        Ok(())
    }

    /// Appends a replayable event to one run's log.
    pub fn append_run_event(&mut self, run_id: &RunId, event: RunEvent) -> Result<RunEventRecord> {
        self.ensure_run_exists(run_id)?;

        let now = now_millis()?;
        let event_type = event.event_name();
        let payload = serde_json::to_string(&event).context("failed to encode runtime event")?;
        self.connection
            .execute(
                "
                INSERT INTO runtime_events (
                    run_id,
                    event_type,
                    payload,
                    created_at
                )
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![run_id.as_str(), event_type, payload, now],
            )
            .context("failed to append runtime event")?;
        let id = self.connection.last_insert_rowid();

        Ok(RunEventRecord {
            id,
            run_id: run_id.clone(),
            event,
            created_at: now,
        })
    }

    /// Loads persisted run events after a cursor.
    pub fn load_run_events_after(
        &self,
        run_id: &RunId,
        after_event_id: Option<i64>,
    ) -> Result<Vec<RunEventRecord>> {
        self.ensure_run_exists(run_id)?;

        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, run_id, payload, created_at
                FROM runtime_events
                WHERE run_id = ?1
                  AND id > ?2
                ORDER BY id ASC
                ",
            )
            .context("failed to prepare runtime event replay")?;

        statement
            .query_map(
                params![run_id.as_str(), after_event_id.unwrap_or(0)],
                |row| {
                    let event: RunEvent =
                        serde_json::from_str(&row.get::<_, String>(2)?).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                2,
                                Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok(RunEventRecord {
                        id: row.get(0)?,
                        run_id: RunId::new(row.get::<_, String>(1)?),
                        event,
                        created_at: row.get(3)?,
                    })
                },
            )
            .context("failed to replay runtime events")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode runtime events")
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
    /// This is public so `perf.rs` can time row loading separately from
    /// part/image attachment while keeping timing ownership outside the store.
    pub fn load_message_rows(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
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
    /// This is public so `perf.rs` can time active-path row loading separately
    /// from active message lookup and part/image attachment.
    /// The recursive step starts from the one-row `path` table and uses
    /// `CROSS JOIN` to keep SQLite on primary-key parent lookups even before a
    /// fresh database has planner statistics.
    pub fn load_path_to_message_rows(
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
            .ok_or_else(|| {
                error::not_found(format!(
                    "message does not exist in conversation: {message_id}"
                ))
            })?;

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

        let mut messages = statement
            .query_map(
                params![conversation_id.as_str(), created_at, rowid],
                read_message_row,
            )
            .context("failed to load messages after checkpoint")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read messages after checkpoint")?;
        self.attach_message_parts(&mut messages)
            .context("failed to load message parts after checkpoint")?;

        Ok(messages)
    }

    /// Attaches ordered text/image parts to already-loaded message rows.
    ///
    /// This is public so `perf.rs` can time part/image attachment separately
    /// from row loading. Callers must pass messages loaded from this store.
    pub fn attach_message_parts(&self, messages: &mut [Message]) -> Result<()> {
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

    /// Loads one image asset only when it is referenced by the conversation.
    ///
    /// Conversation APIs use this as the binary transfer boundary for image
    /// parts. The `message_parts` link keeps ownership scoped to messages in
    /// the requested conversation, so clients cannot fetch an arbitrary asset by
    /// guessing an image ID from another conversation.
    pub fn load_conversation_image_asset(
        &self,
        conversation_id: &ConversationId,
        image_asset_id: &ImageAssetId,
    ) -> Result<ImagePart> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "
                SELECT image_assets.id, image_assets.mime_type, image_assets.bytes
                FROM image_assets
                WHERE image_assets.id = ?2
                  AND EXISTS (
                      SELECT 1
                      FROM message_parts
                      JOIN messages ON messages.id = message_parts.message_id
                      WHERE messages.conversation_id = ?1
                        AND message_parts.image_asset_id = image_assets.id
                  )
                ",
                params![conversation_id.as_str(), image_asset_id.as_str()],
                |row| {
                    Ok(ImagePart {
                        asset_id: ImageAssetId::new(row.get::<_, String>(0)?),
                        mime_type: row.get(1)?,
                        bytes: row.get(2)?,
                    })
                },
            )
            .optional()
            .context("failed to load conversation image asset")?
            .ok_or_else(|| {
                error::not_found(format!(
                    "image asset does not exist in conversation: {image_asset_id}"
                ))
            })
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

        self.insert_message_unchecked(
            conversation_id,
            parent_message_id,
            role,
            content,
            metadata,
            true,
        )
    }

    /// Inserts one runtime-produced message without changing the UI-selected
    /// active message.
    pub fn insert_run_message(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        role: Role,
        content: &str,
        metadata: Option<&MessageMetadata>,
    ) -> Result<MessageId> {
        if role == Role::Tool {
            return Err(error::invalid_request(
                "role: tool messages must be created through insert_run_tool_result_message",
            ));
        }

        self.insert_message_unchecked(
            conversation_id,
            parent_message_id,
            role,
            content,
            metadata,
            false,
        )
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
            true,
        )
    }

    /// Inserts one runtime-produced tool result without changing UI selection.
    pub fn insert_run_tool_result_message(
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
            false,
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
            true,
        )
    }

    /// Inserts a runtime-produced multipart tool result without changing UI
    /// selection.
    pub fn insert_run_tool_result_message_with_parts(
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
            false,
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
        activate: bool,
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

        if activate {
            set_active_message_in_transaction(&transaction, conversation_id, Some(&id))
                .context("failed to set active message")?;
        }
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
            true,
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
        activate: bool,
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
        if activate {
            set_active_message_in_transaction(&transaction, conversation_id, Some(&id))
                .context("failed to set active message")?;
        }
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
                    title,
                    model,
                    reasoning_effort,
                    active_message_id,
                    system_prompt,
                    tool_approval_mode,
                    created_at,
                    updated_at
                )
                VALUES (?1, NULL, ?2, ?3, NULL, NULL, ?4, ?5, ?5)
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
                "DELETE FROM tool_schemas WHERE conversation_id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation tool schemas")?;
        transaction
            .execute(
                "DELETE FROM messages WHERE conversation_id = ?1",
                params![conversation_id.as_str()],
            )
            .context("failed to delete conversation messages")?;
        delete_orphan_image_assets_in_transaction(&transaction)
            .context("failed to delete orphan image assets")?;
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

    /// Returns an error instead of silently ignoring missing runs.
    fn ensure_run_exists(&self, run_id: &RunId) -> Result<()> {
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM runtime_runs WHERE id = ?1",
                params![run_id.as_str()],
                |_| Ok(()),
            )
            .optional()
            .context("failed to check runtime run existence")?
            .is_some();

        if !exists {
            return Err(error::not_found(format!(
                "runtime run does not exist: {run_id}"
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

/// Converts one SQLite runtime run row into the typed run model.
fn run_from_row(row: &Row<'_>) -> rusqlite::Result<Run> {
    let status_text = row.get::<_, String>(4)?;
    let status = RunStatus::from_storage(&status_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown runtime run status: {status_text}"),
            )),
        )
    })?;
    let reasoning_json = row.get::<_, Option<String>>(6)?;
    let reasoning = reasoning_json
        .map(|json| serde_json::from_str::<ReasoningRequest>(&json))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
        })?;

    Ok(Run {
        id: RunId::new(row.get::<_, String>(0)?),
        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
        start_head_message_id: row.get::<_, Option<String>>(2)?.map(MessageId::new),
        current_head_message_id: row.get::<_, Option<String>>(3)?.map(MessageId::new),
        status,
        model: row.get(5)?,
        reasoning,
        error: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
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
#[path = "store_tests.rs"]
mod tests;
