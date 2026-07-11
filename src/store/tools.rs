//! Tools persistence owned by the store module.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Executing,
    Completed,
    Failed,
    Interrupted,
}

impl ToolExecutionStatus {
    fn as_storage(self) -> &'static str {
        match self {
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "executing" => Ok(Self::Executing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(anyhow!("unknown tool execution status: {value}")),
        }
    }
}

impl std::fmt::Display for ToolExecutionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolExecutionRecord {
    pub conversation_id: ConversationId,
    pub assistant_message_id: MessageId,
    pub tool_call_id: ToolCallId,
    pub run_id: String,
    pub status: ToolExecutionStatus,
    pub result_message_id: Option<MessageId>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Store {
    /// Atomically reserves one assistant-requested tool call for execution.
    /// Interrupted claims may be explicitly retried; every other existing claim
    /// prevents the external side effect from running again.
    pub fn claim_tool_call_execution(
        &self,
        conversation_id: &ConversationId,
        assistant_message_id: &MessageId,
        tool_call_id: &ToolCallId,
        run_id: &str,
    ) -> Result<()> {
        self.ensure_message_belongs_to_conversation(conversation_id, assistant_message_id)?;
        let now = now_millis()?;
        let inserted = self
            .connection
            .execute(
                "
                INSERT OR IGNORE INTO tool_call_executions (
                    conversation_id, assistant_message_id, tool_call_id, run_id, status,
                    result_message_id, error, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, 'executing', NULL, NULL, ?5, ?5)
                ",
                params![
                    conversation_id.as_str(),
                    assistant_message_id.as_str(),
                    tool_call_id.as_str(),
                    run_id,
                    now
                ],
            )
            .context("failed to claim tool call execution")?;
        if inserted == 1 {
            return Ok(());
        }

        let retried = self
            .connection
            .execute(
                "
                UPDATE tool_call_executions
                SET run_id = ?3, status = 'executing', error = NULL, updated_at = ?4
                WHERE assistant_message_id = ?1
                  AND tool_call_id = ?2
                  AND status = 'interrupted'
                ",
                params![
                    assistant_message_id.as_str(),
                    tool_call_id.as_str(),
                    run_id,
                    now
                ],
            )
            .context("failed to retry interrupted tool call")?;
        if retried == 1 {
            return Ok(());
        }

        let status = self
            .connection
            .query_row(
                "
                SELECT status
                FROM tool_call_executions
                WHERE assistant_message_id = ?1 AND tool_call_id = ?2
                ",
                params![assistant_message_id.as_str(), tool_call_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .context("failed to load existing tool call execution")?;
        Err(error::invalid_request(format!(
            "tool call execution is already {status}: {tool_call_id}"
        )))
    }

    /// Preserves an execution failure that did not produce a model-facing result.
    pub fn fail_tool_call_execution(
        &self,
        assistant_message_id: &MessageId,
        tool_call_id: &ToolCallId,
        run_id: &str,
        execution_error: &str,
    ) -> Result<()> {
        let changed = self
            .connection
            .execute(
                "
                UPDATE tool_call_executions
                SET status = 'failed', error = ?4, updated_at = ?5
                WHERE assistant_message_id = ?1
                  AND tool_call_id = ?2
                  AND run_id = ?3
                  AND status = 'executing'
                ",
                params![
                    assistant_message_id.as_str(),
                    tool_call_id.as_str(),
                    run_id,
                    execution_error,
                    now_millis()?
                ],
            )
            .context("failed to record tool call execution failure")?;
        if changed != 1 {
            return Err(error::invalid_request(format!(
                "tool call execution is not executing: {tool_call_id}"
            )));
        }
        Ok(())
    }

    /// Stores a tool result and completes its execution claim in one commit.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_tool_call_with_result(
        &mut self,
        conversation_id: &ConversationId,
        assistant_message_id: &MessageId,
        parent_message_id: &MessageId,
        tool_call_id: &ToolCallId,
        run_id: &str,
        content: &str,
        parts: &[UnsavedMessagePart],
    ) -> Result<MessageId> {
        self.ensure_tool_result_parent_matches_call(
            conversation_id,
            parent_message_id,
            tool_call_id,
        )?;
        let id = MessageId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;
        let metadata = MessageMetadata {
            tool_call_id: Some(tool_call_id.clone()),
            ..Default::default()
        };
        let metadata_json = encode_message_metadata(Some(&metadata))?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start atomic tool result transaction")?;
        let executing = transaction
            .query_row(
                "
                SELECT 1
                FROM tool_call_executions
                WHERE assistant_message_id = ?1
                  AND tool_call_id = ?2
                  AND run_id = ?3
                  AND status = 'executing'
                ",
                params![assistant_message_id.as_str(), tool_call_id.as_str(), run_id],
                |_| Ok(()),
            )
            .optional()
            .context("failed to validate executing tool claim")?
            .is_some();
        if !executing {
            return Err(error::invalid_request(format!(
                "tool call execution is not executing for run {run_id}: {tool_call_id}"
            )));
        }

        transaction
            .execute(
                "
                INSERT INTO messages (
                    id, conversation_id, parent_message_id, role, content, metadata, created_at
                ) VALUES (?1, ?2, ?3, 'tool', ?4, ?5, ?6)
                ",
                params![
                    id.as_str(),
                    conversation_id.as_str(),
                    parent_message_id.as_str(),
                    content,
                    metadata_json.as_deref(),
                    now
                ],
            )
            .context("failed to save claimed tool result")?;
        if !parts.is_empty() {
            insert_unsaved_message_parts_in_transaction(&transaction, &id, parts, now)
                .context("failed to save claimed tool result parts")?;
        }
        select_inserted_message(
            &transaction,
            conversation_id,
            &id,
            InsertSelection::IfCurrent(Some(parent_message_id)),
        )?;
        touch_conversation_in_transaction(&transaction, conversation_id, now)?;
        let changed = transaction
            .execute(
                "
                UPDATE tool_call_executions
                SET status = 'completed', result_message_id = ?4, error = NULL, updated_at = ?5
                WHERE assistant_message_id = ?1
                  AND tool_call_id = ?2
                  AND run_id = ?3
                  AND status = 'executing'
                ",
                params![
                    assistant_message_id.as_str(),
                    tool_call_id.as_str(),
                    run_id,
                    id.as_str(),
                    now
                ],
            )
            .context("failed to complete claimed tool result")?;
        if changed != 1 {
            return Err(error::invalid_request(format!(
                "tool call execution changed before completion: {tool_call_id}"
            )));
        }
        transaction
            .commit()
            .context("failed to commit claimed tool result")?;
        Ok(id)
    }

    pub fn tool_execution_records(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<ToolExecutionRecord>> {
        self.ensure_conversation_exists(conversation_id)?;
        let mut statement = self.connection.prepare(
            "
            SELECT conversation_id, assistant_message_id, tool_call_id, run_id,
                   status, result_message_id, error, created_at, updated_at
            FROM tool_call_executions
            WHERE conversation_id = ?1
            ORDER BY created_at, rowid
            ",
        )?;
        let rows = statement.query_map(params![conversation_id.as_str()], |row| {
            let status = row.get::<_, String>(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                status,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })?;
        rows.map(|row| {
            let (
                conversation_id,
                assistant_message_id,
                tool_call_id,
                run_id,
                status,
                result_message_id,
                error,
                created_at,
                updated_at,
            ) = row?;
            Ok(ToolExecutionRecord {
                conversation_id: ConversationId::new(conversation_id),
                assistant_message_id: MessageId::new(assistant_message_id),
                tool_call_id: ToolCallId::new(tool_call_id),
                run_id,
                status: ToolExecutionStatus::from_storage(&status)?,
                result_message_id: result_message_id.map(MessageId::new),
                error,
                created_at,
                updated_at,
            })
        })
        .collect()
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
}

impl Store {
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
}

pub(super) fn read_attached_tool_row(row: &Row<'_>) -> rusqlite::Result<AttachedTool> {
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
