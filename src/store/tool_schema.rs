//! Tree-wide tool capability persistence.
//!
//! One tool set per conversation, same for every branch/head.
//! No parent_message_id, no state machine, no path filtering.

use super::*;

impl Store {
    /// Loads all conversation-wide provider tools.
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

        let tools = statement
            .query_map(params![conversation_id.as_str()], read_attached_tool_row)
            .context("failed to load attached tools")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read attached tools")?;

        Ok(tools)
    }

    /// Loads the conversation-wide model-facing schema subset.
    pub fn load_tool_schemas(&self, conversation_id: &ConversationId) -> Result<Vec<ToolSchema>> {
        Ok(self
            .load_attached_tools(conversation_id)?
            .into_iter()
            .map(|tool| tool.schema())
            .collect())
    }

    /// Loads one conversation-wide attached tool by name.
    pub fn load_attached_tool(
        &self,
        conversation_id: &ConversationId,
        name: &ToolSchemaName,
    ) -> Result<Option<AttachedTool>> {
        self.ensure_conversation_exists(conversation_id)?;

        let tool = self
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

        Ok(tool)
    }

    /// Attaches one provider-backed tool to a conversation (tree-wide).
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

    /// Attaches multiple provider-backed tools atomically (tree-wide).
    pub fn insert_attached_tools(
        &mut self,
        conversation_id: &ConversationId,
        attached_tools: &[AttachedTool],
    ) -> Result<()> {
        self.ensure_conversation_exists(conversation_id)?;
        let mut names = HashSet::new();
        for tool in attached_tools {
            validate_attached_tool(tool)?;
            if !names.insert(tool.schema_name.as_str()) {
                return Err(error::invalid_request(format!(
                    "duplicate tool schema in batch: {}",
                    tool.schema_name
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

        for tool in attached_tools {
            insert_attached_tool_in_transaction(&transaction, conversation_id, tool, now)
                .context("failed to attach tools")?;
        }
        touch_conversation_in_transaction(&transaction, conversation_id, now)
            .context("failed to update conversation timestamp")?;
        transaction
            .commit()
            .context("failed to commit attached tools insert")?;

        Ok(())
    }

    /// Inserts one raw model-facing schema as manual tool (tree-wide).
    pub fn insert_tool_schema(
        &mut self,
        conversation_id: &ConversationId,
        tool_schema: &ToolSchema,
    ) -> Result<()> {
        self.insert_attached_tool(conversation_id, &AttachedTool::manual(tool_schema.clone()))
    }

    /// Updates one existing tool schema, including optional rename (tree-wide).
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

    /// Removes one tool schema from a conversation (tree-wide, hard delete).
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

    fn ensure_tool_schema_exists(
        &self,
        conversation_id: &ConversationId,
        name: &ToolSchemaName,
    ) -> Result<()> {
        if self
            .load_attached_tool(conversation_id, name)?
            .is_none()
        {
            return Err(error::not_found(format!(
                "tool schema does not exist: {name}"
            )));
        }
        Ok(())
    }
}

fn read_attached_tool_row(row: &Row<'_>) -> rusqlite::Result<AttachedTool> {
    let name = row.get::<_, String>(0)?;
    let description = row.get::<_, String>(1)?;
    let parameters_json = row.get::<_, String>(2)?;
    let parameters = serde_json::from_str(&parameters_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(e))
    })?;
    let provider_id = ToolProviderId::new(row.get::<_, String>(3)?);
    let provider_tool_name = ProviderToolName::new(row.get::<_, String>(4)?);
    let provider_kind_text = row.get::<_, String>(5)?;
    let provider_kind = ToolProviderKind::from_storage(&provider_kind_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            Type::Text,
            format!("unknown tool provider kind: {provider_kind_text}").into(),
        )
    })?;
    let permissions_json = row.get::<_, String>(6)?;
    let permissions = serde_json::from_str::<Vec<ToolPermission>>(&permissions_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(e)))?;
    let annotations_json = row.get::<_, String>(7)?;
    let annotations = serde_json::from_str::<ToolAnnotations>(&annotations_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(e)))?;

    Ok(AttachedTool {
        schema_name: ToolSchemaName::new(name),
        description,
        parameters,
        provider: ToolProviderRef::new(provider_id, provider_tool_name, provider_kind),
        permissions,
        annotations,
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

fn encode_tool_permissions(permissions: &[ToolPermission]) -> Result<String> {
    serde_json::to_string(permissions).context("failed to serialize tool permissions")
}

fn encode_tool_annotations(annotations: &ToolAnnotations) -> Result<String> {
    serde_json::to_string(annotations).context("failed to serialize tool annotations")
}

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
