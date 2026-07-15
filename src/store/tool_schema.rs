//! Path-scoped tool capability persistence.

use super::*;

impl Store {
    /// Loads all root-scoped effective provider tools.
    pub fn load_attached_tools(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<AttachedTool>> {
        self.load_attached_tools_for_head(conversation_id, None)
    }

    /// Loads all effective provider tools for an explicit conversation path.
    ///
    /// Tool schemas are not messages, but they are attached to the same message
    /// tree. A row with no parent is visible from every path. A row with a
    /// parent is visible only when that message belongs to the selected path.
    /// For each tool name, the latest visible row decides whether the tool is
    /// present or removed.
    pub fn load_attached_tools_for_head(
        &self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<Vec<AttachedTool>> {
        let path_ids = self.context_path_ids(conversation_id, head_message_id)?;
        let path_set = path_ids.iter().cloned().collect::<HashSet<_>>();
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT
                    parent_message_id,
                    name,
                    description,
                    parameters_json,
                    provider_id,
                    provider_tool_name,
                    provider_kind,
                    permissions_json,
                    annotations_json,
                    state
                FROM tool_schemas
                WHERE conversation_id = ?1
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare attached tool load")?;

        let rows = statement
            .query_map(params![conversation_id.as_str()], read_attached_tool_row)
            .context("failed to load attached tools")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read attached tools")?;
        let mut effective = HashMap::<String, AttachedTool>::new();
        let mut order = Vec::<String>::new();

        for row in rows {
            if !parent_applies(row.parent_message_id.as_deref(), &path_set) {
                continue;
            }
            match row.state.as_str() {
                "present" => {
                    let attached_tool = row.attached_tool.ok_or_else(|| {
                        rusqlite::Error::InvalidColumnType(
                            0,
                            "attached_tool".to_string(),
                            Type::Null,
                        )
                    })?;
                    let name = attached_tool.schema_name.as_str().to_string();
                    if !effective.contains_key(&name) {
                        order.push(name.clone());
                    }
                    effective.insert(name, attached_tool);
                }
                "removed" => {
                    effective.remove(&row.name);
                    order.retain(|name| name != &row.name);
                }
                _ => return Err(anyhow!("unknown tool schema state: {}", row.state)),
            }
        }

        Ok(order
            .into_iter()
            .filter_map(|name| effective.remove(&name))
            .collect())
    }

    /// Loads raw tool-schema rows whose parents are visible from one source path.
    pub(super) fn tool_schema_rows_for_path(
        &self,
        conversation_id: &ConversationId,
        path_ids: &HashSet<String>,
    ) -> Result<Vec<StoredToolSchemaRow>> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT
                    parent_message_id,
                    name,
                    description,
                    parameters_json,
                    provider_id,
                    provider_tool_name,
                    provider_kind,
                    permissions_json,
                    annotations_json,
                    state,
                    created_at
                FROM tool_schemas
                WHERE conversation_id = ?1
                ORDER BY created_at, rowid
                ",
            )
            .context("failed to prepare tool schema row load")?;

        Ok(statement
            .query_map(params![conversation_id.as_str()], |row| {
                Ok(StoredToolSchemaRow {
                    parent_message_id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    parameters_json: row.get(3)?,
                    provider_id: row.get(4)?,
                    provider_tool_name: row.get(5)?,
                    provider_kind: row.get(6)?,
                    permissions_json: row.get(7)?,
                    annotations_json: row.get(8)?,
                    state: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })
            .context("failed to load tool schema rows")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read tool schema rows")?
            .into_iter()
            .filter(|row| parent_applies(row.parent_message_id.as_deref(), path_ids))
            .collect())
    }

    /// Loads the root-scoped effective model-facing schema subset.
    pub fn load_tool_schemas(&self, conversation_id: &ConversationId) -> Result<Vec<ToolSchema>> {
        Ok(self
            .load_attached_tools(conversation_id)?
            .into_iter()
            .map(|tool| tool.schema())
            .collect())
    }

    /// Loads the effective model-facing schema subset for an explicit head.
    pub fn load_tool_schemas_for_head(
        &self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<Vec<ToolSchema>> {
        Ok(self
            .load_attached_tools_for_head(conversation_id, head_message_id)?
            .into_iter()
            .map(|tool| tool.schema())
            .collect())
    }

    /// Loads one effective attached tool by its model-facing schema name.
    pub fn load_attached_tool(
        &self,
        conversation_id: &ConversationId,
        name: &ToolSchemaName,
    ) -> Result<Option<AttachedTool>> {
        self.load_attached_tool_for_head(conversation_id, None, name)
    }

    /// Loads one effective attached tool by schema name for an explicit head.
    pub fn load_attached_tool_for_head(
        &self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
        name: &ToolSchemaName,
    ) -> Result<Option<AttachedTool>> {
        Ok(self
            .load_attached_tools_for_head(conversation_id, head_message_id)?
            .into_iter()
            .find(|tool| &tool.schema_name == name))
    }

    /// Attaches one provider-backed tool to a conversation.
    pub fn insert_attached_tool(
        &mut self,
        conversation_id: &ConversationId,
        attached_tool: &AttachedTool,
    ) -> Result<()> {
        self.insert_attached_tool_at_head(conversation_id, None, attached_tool)
    }

    /// Attaches one provider-backed tool to an explicit conversation path.
    pub fn insert_attached_tool_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        attached_tool: &AttachedTool,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = parent_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }
        validate_attached_tool(attached_tool)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start attached tool insert transaction")?;

        insert_attached_tool_in_transaction(
            &transaction,
            conversation_id,
            parent_message_id,
            attached_tool,
            "present",
            now,
        )
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
        self.insert_attached_tools_at_head(conversation_id, None, attached_tools)
    }

    /// Attaches multiple provider-backed tools to an explicit conversation path.
    pub fn insert_attached_tools_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        attached_tools: &[AttachedTool],
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = parent_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }
        let mut names = HashSet::new();
        for attached_tool in attached_tools {
            validate_attached_tool(attached_tool)?;
            if !names.insert(attached_tool.schema_name.as_str()) {
                return Err(error::invalid_request(format!(
                    "duplicate tool schema in batch: {}",
                    attached_tool.schema_name
                )))
                .context("failed to attach tools");
            }
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
            insert_attached_tool_in_transaction(
                &transaction,
                conversation_id,
                parent_message_id,
                attached_tool,
                "present",
                now,
            )
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

    /// Inserts one raw model-facing schema at an explicit conversation path.
    pub fn insert_tool_schema_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        tool_schema: &ToolSchema,
    ) -> Result<()> {
        self.insert_attached_tool_at_head(
            conversation_id,
            parent_message_id,
            &AttachedTool::manual(tool_schema.clone()),
        )
    }

    /// Updates one existing tool schema, including an optional rename.
    pub fn update_tool_schema(
        &mut self,
        conversation_id: &ConversationId,
        current_name: &ToolSchemaName,
        tool_schema: &ToolSchema,
    ) -> Result<()> {
        self.update_tool_schema_at_head(conversation_id, None, current_name, tool_schema)
    }

    /// Updates one existing tool schema at an explicit conversation path.
    pub fn update_tool_schema_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        current_name: &ToolSchemaName,
        tool_schema: &ToolSchema,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = parent_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }
        self.ensure_tool_schema_exists_at_head(conversation_id, parent_message_id, current_name)?;
        let attached_tool = AttachedTool::manual(tool_schema.clone());
        validate_attached_tool(&attached_tool)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start attached tool update transaction")?;

        if current_name != &attached_tool.schema_name {
            insert_tool_remove_in_transaction(
                &transaction,
                conversation_id,
                parent_message_id,
                current_name,
                now,
            )
            .context("failed to remove renamed attached tool")?;
        }
        insert_attached_tool_in_transaction(
            &transaction,
            conversation_id,
            parent_message_id,
            &attached_tool,
            "present",
            now,
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
        self.remove_tool_schema_at_head(conversation_id, None, name)
    }

    /// Removes one tool schema at an explicit conversation path.
    pub fn remove_tool_schema_at_head(
        &mut self,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        name: &ToolSchemaName,
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = parent_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }
        self.ensure_tool_schema_exists_at_head(conversation_id, parent_message_id, name)?;

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start tool schema delete transaction")?;

        insert_tool_remove_in_transaction(
            &transaction,
            conversation_id,
            parent_message_id,
            name,
            now,
        )
        .context("failed to remove tool schema")?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit tool schema delete")?;

        Ok(())
    }

    /// Returns an error when a tool schema name is not present on an explicit
    /// conversation path.
    fn ensure_tool_schema_exists_at_head(
        &self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
        name: &ToolSchemaName,
    ) -> Result<()> {
        let exists = self
            .load_attached_tool_for_head(conversation_id, head_message_id, name)?
            .is_some();

        if !exists {
            return Err(error::not_found(format!(
                "tool schema does not exist: {name}"
            )));
        }

        Ok(())
    }
}

struct StoredAttachedToolRow {
    parent_message_id: Option<String>,
    name: String,
    state: String,
    attached_tool: Option<AttachedTool>,
}

/// Raw tool-schema row used when copying one path into a fork.
pub(super) struct StoredToolSchemaRow {
    pub(super) parent_message_id: Option<String>,
    pub(super) name: String,
    pub(super) description: Option<String>,
    pub(super) parameters_json: Option<String>,
    pub(super) provider_id: Option<String>,
    pub(super) provider_tool_name: Option<String>,
    pub(super) provider_kind: Option<String>,
    pub(super) permissions_json: Option<String>,
    pub(super) annotations_json: Option<String>,
    pub(super) state: String,
    pub(super) created_at: i64,
}

/// Converts one SQLite tool schema row into an attached tool row.
fn read_attached_tool_row(row: &Row<'_>) -> rusqlite::Result<StoredAttachedToolRow> {
    let parent_message_id = row.get::<_, Option<String>>(0)?;
    let name = row.get::<_, String>(1)?;
    let state = row.get::<_, String>(9)?;
    if state == "removed" {
        return Ok(StoredAttachedToolRow {
            parent_message_id,
            name,
            state,
            attached_tool: None,
        });
    }

    let parameters_json = row.get::<_, Option<String>>(3)?.ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(3, "parameters_json".to_string(), Type::Null)
    })?;
    let parameters = serde_json::from_str(&parameters_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
    })?;
    let provider_kind_text = row.get::<_, Option<String>>(6)?.ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(6, "provider_kind".to_string(), Type::Null)
    })?;
    let provider_kind = ToolProviderKind::from_storage(&provider_kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            Type::Text,
            format!("unknown tool provider kind: {provider_kind_text}").into(),
        )
    })?;
    let permissions_json = row.get::<_, Option<String>>(7)?.ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(7, "permissions_json".to_string(), Type::Null)
    })?;
    let permissions =
        serde_json::from_str::<Vec<ToolPermission>>(&permissions_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(error))
        })?;
    let annotations_json = row.get::<_, Option<String>>(8)?.ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(8, "annotations_json".to_string(), Type::Null)
    })?;
    let annotations =
        serde_json::from_str::<ToolAnnotations>(&annotations_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
        })?;

    Ok(StoredAttachedToolRow {
        parent_message_id,
        name: name.clone(),
        state,
        attached_tool: Some(AttachedTool {
            schema_name: ToolSchemaName::new(name),
            description: row.get::<_, Option<String>>(2)?.ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(2, "description".to_string(), Type::Null)
            })?,
            parameters,
            provider: ToolProviderRef::new(
                ToolProviderId::new(row.get::<_, Option<String>>(4)?.ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(4, "provider_id".to_string(), Type::Null)
                })?),
                ProviderToolName::new(row.get::<_, Option<String>>(5)?.ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        5,
                        "provider_tool_name".to_string(),
                        Type::Null,
                    )
                })?),
                provider_kind,
            ),
            permissions,
            annotations,
        }),
    })
}

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
    parent_message_id: Option<&MessageId>,
    attached_tool: &AttachedTool,
    state: &str,
    now: i64,
) -> Result<()> {
    let parameters_json = encode_tool_parameters(&attached_tool.parameters)?;
    let permissions_json = encode_tool_permissions(&attached_tool.permissions)?;
    let annotations_json = encode_tool_annotations(&attached_tool.annotations)?;
    let id = Uuid::new_v4().to_string();

    transaction.execute(
        "
        INSERT INTO tool_schemas (
            id,
            conversation_id,
            parent_message_id,
            name,
            description,
            parameters_json,
            provider_id,
            provider_tool_name,
            provider_kind,
            permissions_json,
            annotations_json,
            state,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ",
        params![
            id,
            conversation_id.as_str(),
            parent_message_id.map(MessageId::as_str),
            attached_tool.schema_name.as_str(),
            attached_tool.description.as_str(),
            parameters_json.as_str(),
            attached_tool.provider.provider_id.as_str(),
            attached_tool.provider.tool_name.as_str(),
            attached_tool.provider.kind.as_storage(),
            permissions_json.as_str(),
            annotations_json.as_str(),
            state,
            now
        ],
    )?;

    Ok(())
}

/// Copies one raw tool-schema path row into a forked conversation.
pub(super) fn insert_tool_schema_row_in_transaction(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    row: &StoredToolSchemaRow,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();

    transaction.execute(
        "
        INSERT INTO tool_schemas (
            id,
            conversation_id,
            parent_message_id,
            name,
            description,
            parameters_json,
            provider_id,
            provider_tool_name,
            provider_kind,
            permissions_json,
            annotations_json,
            state,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ",
        params![
            id,
            conversation_id.as_str(),
            parent_message_id.map(MessageId::as_str),
            row.name.as_str(),
            row.description.as_deref(),
            row.parameters_json.as_deref(),
            row.provider_id.as_deref(),
            row.provider_tool_name.as_deref(),
            row.provider_kind.as_deref(),
            row.permissions_json.as_deref(),
            row.annotations_json.as_deref(),
            row.state.as_str(),
            row.created_at
        ],
    )?;

    Ok(())
}

/// Appends a path-scoped tool schema removal inside an existing transaction.
fn insert_tool_remove_in_transaction(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    name: &ToolSchemaName,
    now: i64,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();

    transaction.execute(
        "
        INSERT INTO tool_schemas (
            id,
            conversation_id,
            parent_message_id,
            name,
            state,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, 'removed', ?5)
        ",
        params![
            id,
            conversation_id.as_str(),
            parent_message_id.map(MessageId::as_str),
            name.as_str(),
            now
        ],
    )?;

    Ok(())
}

/// Returns whether a path-scoped context row applies to the current head path.
fn parent_applies(parent_message_id: Option<&str>, path_ids: &HashSet<String>) -> bool {
    match parent_message_id {
        Some(message_id) => path_ids.contains(message_id),
        None => true,
    }
}
