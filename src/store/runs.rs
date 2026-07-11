//! Runs persistence owned by the store module.

use super::*;

impl Store {
    /// Creates one backend-owned runtime run for an existing conversation.
    pub fn create_runtime_run(&self, conversation_id: &ConversationId) -> Result<RuntimeRunRecord> {
        if !self.conversation_exists(conversation_id)? {
            return Err(error::not_found(format!(
                "conversation does not exist: {conversation_id}"
            )));
        }

        let record = RuntimeRunRecord {
            id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.clone(),
            status: RuntimeRunStatus::Running,
            error: None,
            created_at: now_millis()?,
            updated_at: now_millis()?,
        };
        if let Err(insert_error) = self.connection.execute(
            "
                INSERT INTO runtime_runs (
                    id, conversation_id, status, error, created_at, updated_at
                ) VALUES (?1, ?2, ?3, NULL, ?4, ?5)
                ",
            params![
                record.id,
                record.conversation_id.as_str(),
                record.status.as_storage(),
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
                SELECT id, conversation_id, status, error, created_at, updated_at
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
                SELECT id, conversation_id, status, error, created_at, updated_at
                FROM runtime_runs
                WHERE conversation_id = ?1 AND status = ?2
                ORDER BY created_at DESC
                LIMIT 1
                ",
                params![
                    conversation_id.as_str(),
                    RuntimeRunStatus::Running.as_storage()
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

    /// Marks work left running by a previous process as interrupted.
    pub fn interrupt_running_runtime_runs(&self) -> Result<()> {
        self.connection
            .execute(
                "
                UPDATE runtime_runs
                SET status = ?1,
                    error = 'Windie stopped before this run completed',
                    updated_at = ?2
                WHERE status = ?3
                ",
                params![
                    RuntimeRunStatus::Interrupted.as_storage(),
                    now_millis()?,
                    RuntimeRunStatus::Running.as_storage()
                ],
            )
            .context("failed to mark interrupted runtime runs")?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Persisted backend-owned runtime run.
pub struct RuntimeRunRecord {
    pub id: String,
    pub conversation_id: ConversationId,
    pub status: RuntimeRunStatus,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
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
    let status = row.get::<_, String>(2)?;
    let status = RuntimeRunStatus::from_storage(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            format!("unknown runtime run status: {status}").into(),
        )
    })?;

    Ok(RuntimeRunRecord {
        id: row.get(0)?,
        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
        status,
        error: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One ordered serialized event emitted by a runtime run.
pub struct RuntimeRunEventRecord {
    pub sequence: u64,
    pub payload: String,
}
