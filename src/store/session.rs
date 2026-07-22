//! Runtime session and replayable session-event persistence.

use super::compaction::delete_compactions_for_conversation;
use super::message::insert_unsaved_message_parts_in_transaction;
use super::*;

use crate::session::SessionInputId;

#[derive(Debug, Clone)]
/// One queued user input ready to be materialized into the conversation tree.
pub struct QueuedSessionInput {
    pub id: SessionInputId,
    pub content: String,
    pub parts: Vec<UnsavedMessagePart>,
}

impl Store {
    /// Persists one prepared user input for FIFO execution by a session.
    pub fn enqueue_session_input(
        &mut self,
        session_id: &SessionId,
        content: &str,
        parts: &[UnsavedMessagePart],
    ) -> Result<SessionInputId> {
        self.ensure_session_exists(session_id)?;
        let input_id = SessionInputId::fresh();
        let parts_json = serde_json::to_string(parts).context("failed to encode queued input")?;
        let now = now_millis()?;

        self.connection
            .execute(
                "
                INSERT INTO session_inputs (id, session_id, content, parts_json, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    input_id.as_str(),
                    session_id.as_str(),
                    content,
                    parts_json,
                    now
                ],
            )
            .context("failed to enqueue session input")?;

        Ok(input_id)
    }

    /// Counts pending inputs for one session.
    pub fn session_input_count(&self, session_id: &SessionId) -> Result<usize> {
        self.ensure_session_exists(session_id)?;
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM session_inputs WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to count queued session inputs")
            .map(|count| count as usize)
    }

    /// Materializes the oldest queued input under the latest session head.
    ///
    /// Message creation, session-head movement, and queue removal share one
    /// SQLite transaction so a claimed input cannot be lost between those
    /// state changes.
    pub fn materialize_next_session_input(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Option<QueuedSessionInput>> {
        let session = self.load_session(session_id)?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start queued input transaction")?;
        let queued = transaction
            .query_row(
                "
                SELECT id, content, parts_json
                FROM session_inputs
                WHERE session_id = ?1
                ORDER BY created_at, rowid
                LIMIT 1
                ",
                params![session_id.as_str()],
                |row| {
                    let parts_json: String = row.get(2)?;
                    let parts = serde_json::from_str(&parts_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
                    })?;
                    Ok(QueuedSessionInput {
                        id: SessionInputId::new(row.get::<_, String>(0)?),
                        content: row.get(1)?,
                        parts,
                    })
                },
            )
            .optional()
            .context("failed to load next queued session input")?;

        let Some(queued) = queued else {
            transaction
                .commit()
                .context("failed to commit empty queued input transaction")?;
            return Ok(None);
        };

        let message_id = MessageId::new(Uuid::new_v4().to_string());
        let now = now_millis()?;
        transaction
            .execute(
                "
                INSERT INTO messages (
                    id, conversation_id, parent_message_id, role, content, metadata, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)
                ",
                params![
                    message_id.as_str(),
                    session.conversation_id.as_str(),
                    session
                        .current_head_message_id
                        .as_ref()
                        .map(MessageId::as_str),
                    Role::User.as_str(),
                    queued.content,
                    now
                ],
            )
            .context("failed to materialize queued session message")?;
        insert_unsaved_message_parts_in_transaction(&transaction, &message_id, &queued.parts, now)
            .context("failed to materialize queued session message parts")?;
        transaction
            .execute(
                "DELETE FROM session_inputs WHERE id = ?1",
                params![queued.id.as_str()],
            )
            .context("failed to remove materialized session input")?;
        transaction
            .execute(
                "
                UPDATE sessions
                SET current_head_message_id = ?1, status = ?2, error = NULL, updated_at = ?3
                WHERE id = ?4
                ",
                params![
                    message_id.as_str(),
                    SessionStatus::Running.as_storage(),
                    now,
                    session_id.as_str()
                ],
            )
            .context("failed to advance session to queued message")?;
        touch_conversation_in_transaction(&transaction, &session.conversation_id, now)
            .context("failed to update conversation after queued message")?;
        transaction
            .commit()
            .context("failed to commit queued session input")?;

        Ok(Some(queued))
    }

    /// Creates one selectable session branch from an explicit conversation head.
    pub fn create_session(
        &mut self,
        session_id: &SessionId,
        conversation_id: &ConversationId,
        start_head_message_id: Option<&MessageId>,
        model: &str,
        reasoning: Option<&ReasoningRequest>,
    ) -> Result<Session> {
        self.create_session_with_status(
            session_id,
            conversation_id,
            start_head_message_id,
            model,
            reasoning,
            SessionStatus::Ready,
        )
    }

    fn create_session_with_status(
        &mut self,
        session_id: &SessionId,
        conversation_id: &ConversationId,
        start_head_message_id: Option<&MessageId>,
        model: &str,
        reasoning: Option<&ReasoningRequest>,
        status: SessionStatus,
    ) -> Result<Session> {
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
            .context("failed to start runtime session transaction")?;

        transaction
            .execute(
                "
                INSERT INTO sessions (
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
                    session_id.as_str(),
                    conversation_id.as_str(),
                    start_head_message_id.map(MessageId::as_str),
                    start_head_message_id.map(MessageId::as_str),
                    status.as_storage(),
                    model,
                    reasoning_json.as_deref(),
                    now
                ],
            )
            .context("failed to create runtime session")?;
        transaction
            .commit()
            .context("failed to commit runtime session create")?;

        self.load_session(session_id)
    }

    /// Loads one sessiontime session by ID.
    pub fn load_session(&self, session_id: &SessionId) -> Result<Session> {
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
                FROM sessions
                WHERE id = ?1
                ",
                params![session_id.as_str()],
                session_from_row,
            )
            .optional()
            .context("failed to load runtime session")?
            .ok_or_else(|| {
                error::not_found(format!("runtime session does not exist: {session_id}"))
            })
    }

    /// Lists all known runtime sessions, newest first.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
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
                FROM sessions
                ORDER BY created_at DESC, id DESC
                ",
            )
            .context("failed to prepare runtime session list")?;

        statement
            .query_map([], session_from_row)
            .context("failed to list runtime sessions")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode runtime sessions")
    }

    /// Lists sessions belonging to one conversation.
    pub fn list_conversation_sessions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<Session>> {
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
                FROM sessions
                WHERE conversation_id = ?1
                ORDER BY created_at DESC, id DESC
                ",
            )
            .context("failed to prepare conversation runtime session list")?;

        statement
            .query_map(params![conversation_id.as_str()], session_from_row)
            .context("failed to list conversation runtime sessions")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode conversation runtime sessions")
    }

    /// Deletes one terminal session and its exclusive message suffix.
    ///
    /// Session rows are pointers into the shared conversation tree. Deleting a
    /// session therefore removes only the path segment after its start head,
    /// and only when no other session still needs those messages. Shared
    /// ancestors and surviving session branches remain visible in the tree.
    pub fn remove_session(&mut self, session_id: &SessionId) -> Result<()> {
        let session = self.load_session(session_id)?;
        if matches!(
            session.status,
            SessionStatus::Running | SessionStatus::WaitingForApproval
        ) {
            return Err(error::invalid_request(
                "cannot delete a running or approval-waiting session; stop it first",
            ));
        }

        let conversation_id = session.conversation_id.clone();
        let current_path = session
            .current_head_message_id
            .as_ref()
            .map(|head| self.load_path_to_message_rows(&conversation_id, head))
            .transpose()?
            .unwrap_or_default();
        let start_index = session.start_head_message_id.as_ref().and_then(|start| {
            current_path
                .iter()
                .position(|message| message.id.as_ref() == Some(start))
        });
        let branch_path = match start_index {
            Some(index) => current_path.into_iter().skip(index + 1).collect::<Vec<_>>(),
            None if session.start_head_message_id.is_none() => current_path,
            None => Vec::new(),
        };

        let surviving_sessions = self
            .list_conversation_sessions(&conversation_id)?
            .into_iter()
            .filter(|other| other.id != *session_id)
            .collect::<Vec<_>>();
        let mut protected_message_ids = HashSet::new();
        for other in surviving_sessions {
            for head in [other.start_head_message_id, other.current_head_message_id]
                .into_iter()
                .flatten()
            {
                for message in self.load_path_to_message_rows(&conversation_id, &head)? {
                    if let Some(message_id) = message.id {
                        protected_message_ids.insert(message_id.as_str().to_string());
                    }
                }
            }
        }

        let candidate_message_ids = branch_path
            .iter()
            .filter_map(|message| message.id.as_ref())
            .map(|message_id| message_id.as_str().to_string())
            .filter(|message_id| !protected_message_ids.contains(message_id))
            .collect::<HashSet<_>>();

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction()
            .context("failed to start session delete transaction")?;

        delete_compactions_for_conversation(&transaction, &conversation_id)
            .context("failed to delete compactions after session delete")?;
        transaction
            .execute(
                "DELETE FROM session_events WHERE session_id = ?1",
                params![session_id.as_str()],
            )
            .context("failed to delete session events")?;
        transaction
            .execute(
                "DELETE FROM sessions WHERE id = ?1",
                params![session_id.as_str()],
            )
            .context("failed to delete session")?;

        // Delete leaves first. A candidate with a surviving child is a shared
        // branch point and must remain in the conversation tree.
        for message_id in branch_path
            .iter()
            .rev()
            .filter_map(|message| message.id.as_ref())
            .map(|message_id| message_id.as_str().to_string())
            .filter(|message_id| candidate_message_ids.contains(message_id))
        {
            transaction
                .execute(
                    "
                    DELETE FROM messages
                    WHERE conversation_id = ?1
                      AND id = ?2
                      AND NOT EXISTS (
                          SELECT 1
                          FROM messages AS child
                          WHERE child.parent_message_id = messages.id
                      )
                    ",
                    params![conversation_id.as_str(), message_id],
                )
                .context("failed to delete exclusive session message")?;
        }

        delete_orphan_image_assets_in_transaction(&transaction)
            .context("failed to delete orphan image assets after session delete")?;
        touch_conversation_in_transaction(&transaction, &conversation_id, now)
            .context("failed to update conversation after session delete")?;
        transaction
            .commit()
            .context("failed to commit session delete")?;

        Ok(())
    }

    /// Updates one session's current message head.
    pub fn update_session_head(
        &mut self,
        session_id: &SessionId,
        head_message_id: Option<&MessageId>,
    ) -> Result<()> {
        let session = self.load_session(session_id)?;
        if let Some(message_id) = head_message_id {
            self.ensure_message_belongs_to_conversation(&session.conversation_id, message_id)?;
        }

        let now = now_millis()?;
        self.connection
            .execute(
                "
                UPDATE sessions
                SET current_head_message_id = ?1,
                    updated_at = ?2
                WHERE id = ?3
                ",
                params![
                    head_message_id.map(MessageId::as_str),
                    now,
                    session_id.as_str()
                ],
            )
            .context("failed to update runtime session head")?;

        Ok(())
    }

    /// Updates one session's lifecycle status.
    pub fn update_session_status(
        &mut self,
        session_id: &SessionId,
        status: SessionStatus,
        error: Option<&str>,
    ) -> Result<()> {
        self.ensure_session_exists(session_id)?;

        let now = now_millis()?;
        self.connection
            .execute(
                "
                UPDATE sessions
                SET status = ?1,
                    error = ?2,
                    updated_at = ?3
                WHERE id = ?4
                ",
                params![status.as_storage(), error, now, session_id.as_str()],
            )
            .context("failed to update runtime session status")?;

        Ok(())
    }

    /// Appends a replayable event to one session's log.
    pub fn append_session_event(
        &mut self,
        session_id: &SessionId,
        event: SessionEvent,
    ) -> Result<SessionEventRecord> {
        self.ensure_session_exists(session_id)?;

        let now = now_millis()?;
        let event_type = event.event_name();
        let payload = serde_json::to_string(&event).context("failed to encode runtime event")?;
        self.connection
            .execute(
                "
                INSERT INTO session_events (
                    session_id,
                    event_type,
                    payload,
                    created_at
                )
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![session_id.as_str(), event_type, payload, now],
            )
            .context("failed to append runtime event")?;
        let id = self.connection.last_insert_rowid();

        Ok(SessionEventRecord {
            id,
            session_id: session_id.clone(),
            event,
            created_at: now,
        })
    }

    /// Loads persisted session events after a cursor.
    pub fn load_session_events_after(
        &self,
        session_id: &SessionId,
        after_event_id: Option<i64>,
    ) -> Result<Vec<SessionEventRecord>> {
        self.ensure_session_exists(session_id)?;

        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, session_id, payload, created_at
                FROM session_events
                WHERE session_id = ?1
                  AND id > ?2
                ORDER BY id ASC
                ",
            )
            .context("failed to prepare runtime event replay")?;

        statement
            .query_map(
                params![session_id.as_str(), after_event_id.unwrap_or(0)],
                |row| {
                    let event: SessionEvent = serde_json::from_str(&row.get::<_, String>(2)?)
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                2,
                                Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok(SessionEventRecord {
                        id: row.get(0)?,
                        session_id: SessionId::new(row.get::<_, String>(1)?),
                        event,
                        created_at: row.get(3)?,
                    })
                },
            )
            .context("failed to replay runtime events")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode runtime events")
    }

    /// Returns an error instead of silently ignoring missing sessions.
    fn ensure_session_exists(&self, session_id: &SessionId) -> Result<()> {
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM sessions WHERE id = ?1",
                params![session_id.as_str()],
                |_| Ok(()),
            )
            .optional()
            .context("failed to check runtime session existence")?
            .is_some();

        if !exists {
            return Err(error::not_found(format!(
                "runtime session does not exist: {session_id}"
            )));
        }

        Ok(())
    }
}

fn session_from_row(row: &Row<'_>) -> rusqlite::Result<Session> {
    let status_text = row.get::<_, String>(4)?;
    let status = SessionStatus::from_storage(&status_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown runtime session status: {status_text}"),
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

    Ok(Session {
        id: SessionId::new(row.get::<_, String>(0)?),
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
