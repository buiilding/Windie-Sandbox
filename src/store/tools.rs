//! Tools persistence owned by the store module.

use super::*;

impl Store {
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
