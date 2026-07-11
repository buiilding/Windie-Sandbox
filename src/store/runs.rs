//! Runs persistence owned by the store module.

use super::{
    Context, ConversationId, OptionalExtension, Result, Row, Serialize, Store, Type, Uuid, error,
    now_millis, params,
};

impl Store {
    /// Creates one backend-owned runtime run for an existing conversation.
    pub(crate) fn create_runtime_run(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<RuntimeRunRecord> {
        let now = now_millis()?;
        self.create_owned_runtime_run(
            conversation_id,
            RuntimeRunAction::Query,
            "store-direct",
            now + 30_000,
        )
    }

    /// Creates one runtime operation owned by a live coordinator lease.
    pub fn create_owned_runtime_run(
        &self,
        conversation_id: &ConversationId,
        action: RuntimeRunAction,
        owner_id: &str,
        lease_expires_at: i64,
    ) -> Result<RuntimeRunRecord> {
        if !self.conversation_exists(conversation_id)? {
            return Err(error::not_found(format!(
                "conversation does not exist: {conversation_id}"
            )));
        }

        let record = RuntimeRunRecord {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.clone(),
            action,
            owner_id: owner_id.to_string(),
            status: RuntimeRunStatus::Running,
            error: None,
            lease_expires_at,
            created_at: now_millis()?,
            updated_at: now_millis()?,
        };
        if let Err(insert_error) = self.connection.execute(
            "
                INSERT INTO runtime_runs (
                    id, conversation_id, action, owner_id, status, error,
                    lease_expires_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8)
                ",
            params![
                record.id,
                record.conversation_id.as_str(),
                record.action.as_storage(),
                record.owner_id,
                record.status.as_storage(),
                record.lease_expires_at,
                record.created_at,
                record.updated_at,
            ],
        ) {
            if self.active_runtime_run(conversation_id)?.is_some() {
                return Err(error::invalid_request(format!(
                    "conversation already has a running action: {conversation_id}"
                )));
            }
            return Err(insert_error).context("failed to create runtime run");
        }

        Ok(record)
    }

    /// Loads one runtime run by ID.
    pub fn runtime_run(&self, run_id: &str) -> Result<RuntimeRunRecord> {
        self.connection
            .query_row(
                "
                SELECT id, conversation_id, action, owner_id, status, error,
                       lease_expires_at, created_at, updated_at
                FROM runtime_runs
                WHERE id = ?1
                ",
                params![run_id],
                read_runtime_run_row,
            )
            .optional()
            .context("failed to load runtime run")?
            .ok_or_else(|| error::not_found(format!("runtime run does not exist: {run_id}")))
    }

    /// Returns the newest running run for a conversation, if one exists.
    pub fn active_runtime_run(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<RuntimeRunRecord>> {
        self.connection
            .query_row(
                "
                SELECT id, conversation_id, action, owner_id, status, error,
                       lease_expires_at, created_at, updated_at
                FROM runtime_runs
                WHERE conversation_id = ?1 AND status = ?2 AND lease_expires_at > ?3
                ORDER BY created_at DESC
                LIMIT 1
                ",
                params![
                    conversation_id.as_str(),
                    RuntimeRunStatus::Running.as_storage(),
                    now_millis()?
                ],
                read_runtime_run_row,
            )
            .optional()
            .context("failed to load active runtime run")
    }

    /// Appends one event and returns its run-local sequence number.
    pub fn append_runtime_run_event(&mut self, run_id: &str, payload: &str) -> Result<u64> {
        let transaction = self
            .connection
            .transaction()
            .context("failed to start runtime event transaction")?;
        let status = transaction
            .query_row(
                "SELECT status FROM runtime_runs WHERE id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("failed to load runtime run status")?
            .ok_or_else(|| error::not_found(format!("runtime run does not exist: {run_id}")))?;
        if status != RuntimeRunStatus::Running.as_storage() {
            return Err(error::invalid_request(format!(
                "runtime run is not running: {run_id} ({status})"
            )));
        }
        let next = transaction
            .query_row(
                "
                SELECT COALESCE(MAX(sequence), 0) + 1
                FROM runtime_run_events
                WHERE run_id = ?1
                ",
                params![run_id],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to determine runtime event sequence")?;
        let now = now_millis()?;
        transaction
            .execute(
                "
                INSERT INTO runtime_run_events (run_id, sequence, payload_json, created_at)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![run_id, next, payload, now],
            )
            .context("failed to append runtime event")?;
        transaction
            .execute(
                "UPDATE runtime_runs SET updated_at = ?2 WHERE id = ?1",
                params![run_id, now],
            )
            .context("failed to update runtime run timestamp")?;
        transaction
            .commit()
            .context("failed to commit runtime event")?;

        u64::try_from(next).context("runtime event sequence was negative")
    }

    /// Atomically records one terminal event and changes a running run to its
    /// final status. A competing terminal transition wins by changing the status
    /// first; later transitions return `None` without appending another event.
    pub fn finish_runtime_run(
        &mut self,
        run_id: &str,
        status: RuntimeRunStatus,
        error_message: Option<&str>,
        terminal_payload: &str,
    ) -> Result<Option<u64>> {
        if status == RuntimeRunStatus::Running {
            return Err(error::invalid_request(
                "terminal runtime status must not be running",
            ));
        }

        let transaction = self
            .connection
            .transaction()
            .context("failed to start runtime completion transaction")?;
        let now = now_millis()?;
        let changed = transaction
            .execute(
                "
                UPDATE runtime_runs
                SET status = ?2, error = ?3, updated_at = ?4
                WHERE id = ?1 AND status = ?5
                ",
                params![
                    run_id,
                    status.as_storage(),
                    error_message,
                    now,
                    RuntimeRunStatus::Running.as_storage()
                ],
            )
            .context("failed to finish runtime run")?;
        if changed == 0 {
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM runtime_runs WHERE id = ?1",
                    params![run_id],
                    |_| Ok(()),
                )
                .optional()
                .context("failed to check runtime run")?
                .is_some();
            if !exists {
                return Err(error::not_found(format!(
                    "runtime run does not exist: {run_id}"
                )));
            }
            return Ok(None);
        }

        transaction
            .execute(
                "
                UPDATE tool_call_executions
                SET status = 'unknown',
                    error = 'runtime ended before the tool result was durably recorded',
                    updated_at = ?2
                WHERE run_id = ?1 AND status = 'executing'
                ",
                params![run_id, now],
            )
            .context("failed to reconcile unfinished tool executions")?;

        let next = transaction
            .query_row(
                "
                SELECT COALESCE(MAX(sequence), 0) + 1
                FROM runtime_run_events
                WHERE run_id = ?1
                ",
                params![run_id],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to determine terminal event sequence")?;
        transaction
            .execute(
                "
                INSERT INTO runtime_run_events (run_id, sequence, payload_json, created_at)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![run_id, next, terminal_payload, now],
            )
            .context("failed to append terminal runtime event")?;
        transaction
            .commit()
            .context("failed to commit runtime completion")?;

        Ok(Some(
            u64::try_from(next).context("terminal event sequence was negative")?,
        ))
    }

    /// Loads ordered events strictly after one sequence number.
    pub fn runtime_run_events_after(
        &self,
        run_id: &str,
        after: u64,
    ) -> Result<Vec<RuntimeRunEventRecord>> {
        self.runtime_run(run_id)?;
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT sequence, payload_json
                FROM runtime_run_events
                WHERE run_id = ?1 AND sequence > ?2
                ORDER BY sequence
                ",
            )
            .context("failed to prepare runtime event load")?;
        let rows = statement
            .query_map(params![run_id, after], |row| {
                let sequence = row.get::<_, i64>(0)?;
                Ok(RuntimeRunEventRecord {
                    sequence: u64::try_from(sequence).unwrap_or(0),
                    payload: row.get(1)?,
                })
            })
            .context("failed to load runtime events")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to decode runtime events")
    }

    /// Renews every running operation owned by one live coordinator.
    pub fn renew_runtime_run_leases(&self, owner_id: &str, lease_expires_at: i64) -> Result<()> {
        self.connection
            .execute(
                "
                UPDATE runtime_runs
                SET lease_expires_at = ?2, updated_at = ?3
                WHERE owner_id = ?1 AND status = 'running'
                ",
                params![owner_id, lease_expires_at, now_millis()?],
            )
            .context("failed to renew runtime operation leases")?;
        Ok(())
    }

    /// Marks expired operations abandoned so new work can acquire ownership.
    pub fn interrupt_expired_runtime_runs(&mut self, now: i64) -> Result<()> {
        let transaction = self
            .connection
            .transaction()
            .context("failed to start expired runtime recovery transaction")?;
        transaction
            .execute(
                "
                UPDATE runtime_runs
                SET status = 'interrupted',
                    error = 'Windie runtime ownership lease expired',
                    updated_at = ?1
                WHERE status = 'running' AND lease_expires_at <= ?1
                ",
                params![now],
            )
            .context("failed to interrupt expired runtime operations")?;
        transaction
            .execute(
                "
                UPDATE tool_call_executions
                SET status = 'unknown',
                    error = 'runtime ownership expired before the tool result was durably recorded',
                    updated_at = ?1
                WHERE status = 'executing'
                  AND run_id IN (
                      SELECT id FROM runtime_runs
                      WHERE status = 'interrupted'
                        AND error = 'Windie runtime ownership lease expired'
                        AND updated_at = ?1
                  )
                ",
                params![now],
            )
            .context("failed to reconcile expired tool executions")?;
        transaction
            .commit()
            .context("failed to commit expired runtime recovery")?;
        Ok(())
    }

    /// Marks operations abandoned when their coordinator shuts down cleanly.
    pub fn interrupt_runtime_runs_for_owner(&mut self, owner_id: &str) -> Result<()> {
        let transaction = self
            .connection
            .transaction()
            .context("failed to start runtime owner shutdown transaction")?;
        let now = now_millis()?;
        transaction
            .execute(
                "
                UPDATE runtime_runs
                SET status = 'interrupted',
                    error = 'Windie runtime coordinator stopped',
                    updated_at = ?2
                WHERE owner_id = ?1 AND status = 'running'
                ",
                params![owner_id, now],
            )
            .context("failed to interrupt owned runtime operations")?;
        transaction
            .execute(
                "
                UPDATE tool_call_executions
                SET status = 'unknown',
                    error = 'runtime coordinator stopped before the tool result was durably recorded',
                    updated_at = ?2
                WHERE status = 'executing'
                  AND run_id IN (
                      SELECT id FROM runtime_runs
                      WHERE owner_id = ?1 AND status = 'interrupted'
                  )
                ",
                params![owner_id, now],
            )
            .context("failed to reconcile stopped-owner tool executions")?;
        transaction
            .commit()
            .context("failed to commit runtime owner shutdown")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Persisted backend-owned runtime run.
pub struct RuntimeRunRecord {
    pub id: String,
    pub conversation_id: ConversationId,
    pub action: RuntimeRunAction,
    pub owner_id: String,
    pub status: RuntimeRunStatus,
    pub error: Option<String>,
    pub lease_expires_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Concrete operation owned by one runtime run.
pub enum RuntimeRunAction {
    Query,
    ApproveTool,
    DenyTool,
}

impl RuntimeRunAction {
    /// Returns the SQLite representation for this action.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::ApproveTool => "approve_tool",
            Self::DenyTool => "deny_tool",
        }
    }

    fn from_storage(value: &str) -> Option<Self> {
        match value {
            "query" => Some(Self::Query),
            "approve_tool" => Some(Self::ApproveTool),
            "deny_tool" => Some(Self::DenyTool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Persisted lifecycle state for a backend-owned runtime run.
pub enum RuntimeRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RuntimeRunStatus {
    /// Returns the SQLite representation for this status.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    /// Decodes a persisted runtime run status.
    fn from_storage(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

impl std::fmt::Display for RuntimeRunStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

fn read_runtime_run_row(row: &Row<'_>) -> rusqlite::Result<RuntimeRunRecord> {
    let action = row.get::<_, String>(2)?;
    let action = RuntimeRunAction::from_storage(&action).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            format!("unknown runtime run action: {action}").into(),
        )
    })?;
    let status = row.get::<_, String>(4)?;
    let status = RuntimeRunStatus::from_storage(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            Type::Text,
            format!("unknown runtime run status: {status}").into(),
        )
    })?;

    Ok(RuntimeRunRecord {
        id: row.get(0)?,
        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
        action,
        owner_id: row.get(3)?,
        status,
        error: row.get(5)?,
        lease_expires_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One ordered serialized event emitted by a runtime run.
pub struct RuntimeRunEventRecord {
    pub sequence: u64,
    pub payload: String,
}
