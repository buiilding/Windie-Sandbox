//! Runtime session and replayable session-event persistence.

use super::compaction::delete_compactions_for_conversation;
use super::message::{
    MessageInsert, insert_message_in_transaction, insert_unsaved_message_parts_in_transaction,
};
use super::*;

use crate::session::{SessionExecutionStart, SessionInputId};

const GLOBAL_SESSION_EVENT_BATCH_SIZE: i64 = 256;

#[derive(Debug, Clone)]
/// One queued user input ready to be materialized into the conversation tree.
pub struct QueuedSessionInput {
    pub id: SessionInputId,
    pub content: String,
    pub parts: Vec<UnsavedMessagePart>,
}

/// Wakeup and runtime message variants that advance a durable session branch.
///
/// The store derives the persisted role, tool-call metadata, and replay event
/// from this typed input so callers cannot combine mismatched values.
pub(crate) enum SessionRuntimeMessage<'a> {
    User {
        parent_message_id: Option<&'a MessageId>,
        content: &'a str,
        parts: &'a [UnsavedMessagePart],
    },
    Assistant {
        parent_message_id: Option<&'a MessageId>,
        content: &'a str,
        metadata: Option<&'a MessageMetadata>,
    },
    ToolResult {
        parent_message_id: &'a MessageId,
        tool_call_id: &'a ToolCallId,
        content: &'a str,
        parts: &'a [UnsavedMessagePart],
    },
}

#[derive(Debug)]
/// Result of atomically committing one message through a session claim.
pub(crate) struct SessionRuntimeMessageCommit {
    pub message_id: MessageId,
    pub event: Option<SessionEventRecord>,
}

/// Fully resolved values used by the shared transactional write.
struct SessionMessageInsert<'a> {
    session_id: &'a SessionId,
    claim: &'a SessionExecutionClaim,
    conversation_id: &'a ConversationId,
    parent_message_id: Option<&'a MessageId>,
    role: Role,
    content: &'a str,
    parts: &'a [UnsavedMessagePart],
    metadata: Option<&'a MessageMetadata>,
}

impl Store {
    /// Returns the message IDs on a live session's current inference path.
    ///
    /// This is the persisted protection set exposed to API clients. Keeping
    /// the path calculation here prevents clients from reconstructing session
    /// safety rules from a conversation tree that may already be stale.
    pub fn protected_message_ids_for_session(&self, session: &Session) -> Result<Vec<String>> {
        if !matches!(
            session.status,
            SessionStatus::Running | SessionStatus::WaitingForApproval
        ) {
            return Ok(Vec::new());
        }

        let Some(head_message_id) = session.current_head_message_id.as_ref() else {
            return Ok(Vec::new());
        };

        self.load_path_to_message_rows(&session.conversation_id, head_message_id)
            .map(|path| {
                path.into_iter()
                    .filter_map(|message| message.id.map(|id| id.as_str().to_string()))
                    .collect()
            })
    }

    /// Rejects destructive message mutations that would change the model path
    /// owned by an active session.
    ///
    /// Running and approval-waiting sessions both retain an execution branch:
    /// running sessions may still be using the path for inference, while
    /// approval-waiting sessions still have a pending tool call whose parent
    /// path must remain valid. New children and copied forks do not call this
    /// guard because they leave the protected messages unchanged.
    pub(super) fn ensure_message_mutation_allowed(
        &self,
        conversation_id: &ConversationId,
        affected_message_ids: &HashSet<String>,
    ) -> Result<()> {
        if affected_message_ids.is_empty() {
            return Ok(());
        }

        for session in self.list_conversation_sessions(conversation_id)? {
            if !matches!(
                session.status,
                SessionStatus::Running | SessionStatus::WaitingForApproval
            ) {
                continue;
            }

            let protected_message_id = self
                .protected_message_ids_for_session(&session)?
                .into_iter()
                .find(|message_id| affected_message_ids.contains(message_id));
            let Some(protected_message_id) = protected_message_id else {
                continue;
            };

            return Err(error::conflict(format!(
                "cannot modify message {protected_message_id}; it is part of active session {}",
                session.id
            )));
        }

        Ok(())
    }

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
        claim: &SessionExecutionClaim,
    ) -> Result<Option<(QueuedSessionInput, SessionEventRecord)>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to start queued input transaction")?;
        let (conversation_id, current_head_message_id) = transaction
            .query_row(
                "
                SELECT conversation_id, current_head_message_id
                FROM sessions
                WHERE id = ?1
                  AND status = ?2
                  AND execution_owner = ?3
                  AND execution_claim_id = ?4
                ",
                params![
                    session_id.as_str(),
                    SessionStatus::Running.as_storage(),
                    claim.owner.as_storage(),
                    claim.id.as_str(),
                ],
                |row| {
                    Ok((
                        ConversationId::new(row.get::<_, String>(0)?),
                        row.get::<_, Option<String>>(1)?.map(MessageId::new),
                    ))
                },
            )
            .optional()
            .context("failed to validate queued input execution claim")?
            .ok_or_else(|| {
                error::conflict(
                    "session execution claim was cancelled or transferred before queued input persistence",
                )
            })?;
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
                    conversation_id.as_str(),
                    current_head_message_id.as_ref().map(MessageId::as_str),
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
                SET current_head_message_id = ?1,
                    updated_at = ?2
                WHERE id = ?3
                  AND status = ?4
                  AND execution_owner = ?5
                  AND execution_claim_id = ?6
                ",
                params![
                    message_id.as_str(),
                    now,
                    session_id.as_str(),
                    SessionStatus::Running.as_storage(),
                    claim.owner.as_storage(),
                    claim.id.as_str(),
                ],
            )
            .context("failed to advance session to queued message")?;
        touch_conversation_in_transaction(&transaction, &conversation_id, now)
            .context("failed to update conversation after queued message")?;
        let record = append_session_event_in_transaction(
            &transaction,
            session_id,
            SessionEvent::InputStarted {
                input_id: queued.id.as_str().to_string(),
                message_id: message_id.as_str().to_string(),
            },
            now,
        )?;
        transaction
            .commit()
            .context("failed to commit queued session input")?;

        Ok(Some((queued, record)))
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

    /// Resolves the unique session branch currently ending at a conversation head.
    pub fn resolve_session_at_head(
        &self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
    ) -> Result<SessionResolution> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = head_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }

        Ok(session_resolution_from_matches(sessions_at_head(
            &self.connection,
            conversation_id,
            head_message_id,
        )?))
    }

    /// Resolves or creates one session branch atomically at a conversation head.
    ///
    /// The immediate SQLite transaction serializes competing clients that are
    /// both trying to create the first session at the same branch point. An
    /// ambiguous existing head is rejected instead of silently selecting one.
    pub fn resolve_or_create_session(
        &mut self,
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
        model: &str,
        reasoning: Option<&ReasoningRequest>,
    ) -> Result<Session> {
        self.ensure_conversation_exists(conversation_id)?;
        if let Some(message_id) = head_message_id {
            self.ensure_message_belongs_to_conversation(conversation_id, message_id)?;
        }

        let matches = {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .context("failed to start session resolution transaction")?;
            let matches = sessions_at_head(&transaction, conversation_id, head_message_id)?;
            match matches.len() {
                0 => {
                    let session_id = SessionId::fresh();
                    let now = now_millis()?;
                    let reasoning_json = reasoning
                        .map(serde_json::to_string)
                        .transpose()
                        .context("failed to encode run reasoning")?;
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
                            VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?6, NULL, ?7, ?7)
                            ",
                            params![
                                session_id.as_str(),
                                conversation_id.as_str(),
                                head_message_id.map(MessageId::as_str),
                                head_message_id.map(MessageId::as_str),
                                model,
                                reasoning_json.as_deref(),
                                now,
                            ],
                        )
                        .context("failed to atomically create runtime session")?;
                    transaction
                        .commit()
                        .context("failed to commit resolved runtime session")?;
                    return self.load_session(&session_id);
                }
                1 => {
                    transaction
                        .commit()
                        .context("failed to commit existing session resolution")?;
                    return Ok(matches.into_iter().next().expect("one session match"));
                }
                _ => matches,
            }
        };

        let ids = matches
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Err(crate::error::conflict(format!(
            "multiple sessions exist at conversation head: {ids}"
        )))
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

    /// Atomically persists a session message and advances its session head.
    ///
    /// Assistant and tool-result messages also append their matching replay
    /// event. User inputs have no saved-message event, but still require the
    /// exact execution claim so cancellation fences the entire run.
    pub(crate) fn insert_session_runtime_message(
        &mut self,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        conversation_id: &ConversationId,
        message: SessionRuntimeMessage<'_>,
    ) -> Result<SessionRuntimeMessageCommit> {
        match message {
            SessionRuntimeMessage::User {
                parent_message_id,
                content,
                parts,
            } => self.insert_session_message(SessionMessageInsert {
                session_id,
                claim,
                conversation_id,
                parent_message_id,
                role: Role::User,
                content,
                parts,
                metadata: None,
            }),
            SessionRuntimeMessage::Assistant {
                parent_message_id,
                content,
                metadata,
            } => self.insert_session_message(SessionMessageInsert {
                session_id,
                claim,
                conversation_id,
                parent_message_id,
                role: Role::Assistant,
                content,
                parts: &[],
                metadata,
            }),
            SessionRuntimeMessage::ToolResult {
                parent_message_id,
                tool_call_id,
                content,
                parts,
            } => {
                self.ensure_tool_result_parent_matches_call(
                    conversation_id,
                    parent_message_id,
                    tool_call_id,
                )?;
                let metadata = MessageMetadata {
                    tool_call_id: Some(tool_call_id.clone()),
                    ..Default::default()
                };
                self.insert_session_message(SessionMessageInsert {
                    session_id,
                    claim,
                    conversation_id,
                    parent_message_id: Some(parent_message_id),
                    role: Role::Tool,
                    content,
                    parts,
                    metadata: Some(&metadata),
                })
            }
        }
    }

    /// Commits one runtime-produced message and every durable session pointer
    /// to that message as one SQLite unit.
    ///
    /// The session's current head must still equal the requested parent. This
    /// prevents a stale runtime writer from moving an already-advanced branch.
    fn insert_session_message(
        &mut self,
        message: SessionMessageInsert<'_>,
    ) -> Result<SessionRuntimeMessageCommit> {
        self.ensure_conversation_exists(message.conversation_id)?;
        if let Some(parent_message_id) = message.parent_message_id {
            self.ensure_message_belongs_to_conversation(
                message.conversation_id,
                parent_message_id,
            )?;
        }

        let message_id = MessageId::new(Uuid::new_v4().to_string());
        let metadata_json = encode_message_metadata(message.metadata)?;
        let now = now_millis()?;
        let event = match message.role {
            Role::User => None,
            Role::Assistant => Some(SessionEvent::AssistantMessageSaved {
                message_id: message_id.as_str().to_string(),
            }),
            Role::Tool => Some(SessionEvent::ToolResultSaved {
                message_id: message_id.as_str().to_string(),
            }),
            _ => {
                return Err(error::invalid_request(
                    "session runtime persistence only accepts user, assistant, or tool messages",
                ));
            }
        };

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to start atomic session message transaction")?;
        let (
            session_conversation_id,
            current_head_message_id,
            status,
            execution_owner,
            execution_claim_id,
        ) = transaction
            .query_row(
                "
                SELECT conversation_id, current_head_message_id, status, execution_owner,
                       execution_claim_id
                FROM sessions
                WHERE id = ?1
                ",
                params![message.session_id.as_str()],
                |row| {
                    Ok((
                        ConversationId::new(row.get::<_, String>(0)?),
                        row.get::<_, Option<String>>(1)?.map(MessageId::new),
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .context("failed to load session for atomic message save")?
            .ok_or_else(|| {
                error::not_found(format!(
                    "runtime session does not exist: {}",
                    message.session_id
                ))
            })?;

        if session_conversation_id != *message.conversation_id {
            return Err(error::invalid_request(format!(
                "session does not belong to conversation: {}",
                message.session_id
            )));
        }
        if current_head_message_id.as_ref() != message.parent_message_id {
            return Err(error::conflict(
                "session head changed before runtime message persistence; reload and retry",
            ));
        }
        if status != SessionStatus::Running.as_storage()
            || execution_owner.as_deref() != Some(message.claim.owner.as_storage())
            || execution_claim_id.as_deref() != Some(message.claim.id.as_str())
        {
            return Err(error::conflict(
                "session execution claim was cancelled or transferred before runtime message persistence",
            ));
        }

        insert_message_in_transaction(
            &transaction,
            MessageInsert {
                id: &message_id,
                conversation_id: message.conversation_id,
                parent_message_id: message.parent_message_id,
                role: message.role,
                content: message.content,
                parts: message.parts,
                metadata_json: metadata_json.as_deref(),
                created_at: now,
            },
        )?;
        transaction
            .execute(
                "
                UPDATE sessions
                SET current_head_message_id = ?1,
                    updated_at = ?2
                WHERE id = ?3
                ",
                params![message_id.as_str(), now, message.session_id.as_str()],
            )
            .context("failed to update session head for runtime message")?;
        let record = event
            .map(|event| {
                append_session_event_in_transaction(&transaction, message.session_id, event, now)
            })
            .transpose()?;
        transaction
            .commit()
            .context("failed to commit atomic session message")?;

        Ok(SessionRuntimeMessageCommit {
            message_id,
            event: record,
        })
    }

    /// Atomically claims a session when its durable state matches `start`.
    ///
    /// This is the cross-process execution gate shared by the API supervisor
    /// and CLI. Keeping all valid start conditions in this typed entry point
    /// prevents clients from growing separate claim workflows.
    pub fn claim_session_execution(
        &mut self,
        session_id: &SessionId,
        owner: SessionExecutionOwner,
        start: SessionExecutionStart,
    ) -> Result<ClaimedSession> {
        self.ensure_session_exists(session_id)?;

        let claim = SessionExecutionClaim::fresh(owner);
        let now = now_millis()?;
        let (requires_waiting, requires_head, expected_head, conflict_message) = match &start {
            SessionExecutionStart::Runnable => (
                false,
                false,
                None,
                "session is already running or waiting for approval",
            ),
            SessionExecutionStart::RunnableAtHead(expected_head) => (
                false,
                true,
                expected_head.as_ref().map(MessageId::as_str),
                "session changed while claiming execution; reload and retry",
            ),
            SessionExecutionStart::WaitingForApproval => (
                true,
                false,
                None,
                "session is no longer waiting for approval",
            ),
        };
        let changed = self
            .connection
            .execute(
                "
                UPDATE sessions
                SET status = ?1,
                    error = NULL,
                    execution_owner = ?2,
                    execution_claim_id = ?3,
                    updated_at = ?4
                WHERE id = ?5
                  AND execution_owner IS NULL
                  AND execution_claim_id IS NULL
                  AND (
                      (?6 = 1 AND status = ?7)
                      OR (?6 = 0 AND status NOT IN (?1, ?7))
                  )
                  AND (?8 = 0 OR current_head_message_id IS ?9)
                ",
                params![
                    SessionStatus::Running.as_storage(),
                    owner.as_storage(),
                    claim.id.as_str(),
                    now,
                    session_id.as_str(),
                    requires_waiting,
                    SessionStatus::WaitingForApproval.as_storage(),
                    requires_head,
                    expected_head,
                ],
            )
            .context("failed to claim runtime session execution")?;

        if changed == 0 {
            return Err(error::conflict(conflict_message));
        }

        Ok(ClaimedSession {
            session: self.load_session(session_id)?,
            claim,
        })
    }

    /// Appends one runtime event only while the caller still owns the durable
    /// execution claim. Stream deltas use this guard to stop a cancelled runner
    /// from publishing additional work.
    pub fn append_session_execution_event(
        &mut self,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        event: SessionEvent,
    ) -> Result<SessionEventRecord> {
        self.ensure_session_exists(session_id)?;
        let now = now_millis()?;
        let event_type = event.event_name();
        let payload = serde_json::to_string(&event).context("failed to encode runtime event")?;
        let changed = self
            .connection
            .execute(
                "
                INSERT INTO session_events (session_id, event_type, payload, created_at)
                SELECT ?1, ?2, ?3, ?4
                WHERE EXISTS (
                    SELECT 1
                    FROM sessions
                    WHERE id = ?1
                      AND status = ?5
                      AND execution_owner = ?6
                      AND execution_claim_id = ?7
                )
                ",
                params![
                    session_id.as_str(),
                    event_type,
                    payload,
                    now,
                    SessionStatus::Running.as_storage(),
                    claim.owner.as_storage(),
                    claim.id.as_str(),
                ],
            )
            .context("failed to append claimed runtime event")?;
        if changed == 0 {
            return Err(error::conflict(
                "session execution claim was cancelled or transferred before runtime event persistence",
            ));
        }

        Ok(SessionEventRecord {
            id: self.connection.last_insert_rowid(),
            session_id: session_id.clone(),
            event,
            created_at: now,
        })
    }

    /// Moves a claimed running session to a terminal or paused status.
    ///
    /// The conditional update deliberately refuses to overwrite a durable
    /// cancellation. A runner that finishes after another client cancelled it
    /// must not resurrect the session as completed or failed.
    pub fn finish_claimed_session_execution(
        &mut self,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        status: SessionStatus,
        error: Option<&str>,
    ) -> Result<bool> {
        debug_assert!(matches!(
            status,
            SessionStatus::Completed | SessionStatus::WaitingForApproval | SessionStatus::Failed
        ));
        self.ensure_session_exists(session_id)?;

        let now = now_millis()?;
        let changed = self
            .connection
            .execute(
                "
                UPDATE sessions
                SET status = ?1,
                    error = ?2,
                    execution_owner = NULL,
                    execution_claim_id = NULL,
                    updated_at = ?3
                WHERE id = ?4
                  AND status = ?5
                  AND execution_owner = ?6
                  AND execution_claim_id = ?7
                ",
                params![
                    status.as_storage(),
                    error,
                    now,
                    session_id.as_str(),
                    SessionStatus::Running.as_storage(),
                    claim.owner.as_storage(),
                    claim.id.as_str(),
                ],
            )
            .context("failed to finish claimed runtime session execution")?;

        Ok(changed == 1)
    }

    /// Atomically finishes a claimed execution, moves its head, and records
    /// the terminal event.
    ///
    /// Releasing the claim, publishing the new head, and appending the event
    /// must be one SQLite transaction. Otherwise another process could claim
    /// the terminal session between those writes and start from a stale head.
    pub fn finish_claimed_session_execution_at_head(
        &mut self,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        status: SessionStatus,
        head_message_id: Option<&MessageId>,
        event: SessionEvent,
    ) -> Result<Option<SessionEventRecord>> {
        debug_assert!(matches!(
            status,
            SessionStatus::Completed | SessionStatus::WaitingForApproval
        ));
        let session = self.load_session(session_id)?;
        if let Some(message_id) = head_message_id {
            self.ensure_message_belongs_to_conversation(&session.conversation_id, message_id)?;
        }

        let now = now_millis()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to start claimed session finish transaction")?;
        let changed = transaction
            .execute(
                "
                UPDATE sessions
                SET current_head_message_id = ?1,
                    status = ?2,
                    error = NULL,
                    execution_owner = NULL,
                    execution_claim_id = NULL,
                    updated_at = ?3
                WHERE id = ?4
                  AND status = ?5
                  AND execution_owner = ?6
                  AND execution_claim_id = ?7
                ",
                params![
                    head_message_id.map(MessageId::as_str),
                    status.as_storage(),
                    now,
                    session_id.as_str(),
                    SessionStatus::Running.as_storage(),
                    claim.owner.as_storage(),
                    claim.id.as_str(),
                ],
            )
            .context("failed to finish claimed runtime session at head")?;

        if changed == 0 {
            transaction
                .commit()
                .context("failed to commit unchanged claimed session finish")?;
            return Ok(None);
        }

        let record = append_session_event_in_transaction(&transaction, session_id, event, now)?;
        transaction
            .commit()
            .context("failed to commit claimed session finish")?;
        Ok(Some(record))
    }

    /// Releases a cancelled session's claim after its owner has stopped.
    ///
    /// Cancellation deliberately keeps the owner claim in place until the
    /// runner acknowledges it. That prevents another client from starting the
    /// same session while the original process may still be unwinding.
    pub fn release_cancelled_session_execution(
        &mut self,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
    ) -> Result<()> {
        self.ensure_session_exists(session_id)?;
        let now = now_millis()?;
        self.connection
            .execute(
                "
                UPDATE sessions
                SET execution_owner = NULL,
                    execution_claim_id = NULL,
                    updated_at = ?1
                WHERE id = ?2
                  AND status = ?3
                  AND execution_owner = ?4
                  AND execution_claim_id = ?5
                ",
                params![
                    now,
                    session_id.as_str(),
                    SessionStatus::Cancelled.as_storage(),
                    claim.owner.as_storage(),
                    claim.id.as_str(),
                ],
            )
            .context("failed to release cancelled runtime session execution")?;
        Ok(())
    }

    /// Returns the durable owner of a currently claimed session, if any.
    pub fn session_execution_claim(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionExecutionClaim>> {
        self.ensure_session_exists(session_id)?;
        let claim = self
            .connection
            .query_row(
                "SELECT execution_owner, execution_claim_id FROM sessions WHERE id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .context("failed to load runtime session execution claim")?;

        match claim {
            (None, None) => Ok(None),
            (Some(owner), Some(id)) => Ok(Some(SessionExecutionClaim {
                owner: SessionExecutionOwner::from_storage(&owner)
                    .ok_or_else(|| anyhow!("unknown runtime session execution owner: {owner}"))?,
                id: SessionExecutionClaimId::new(id),
            })),
            _ => Err(anyhow!(
                "runtime session execution claim has incomplete persisted state"
            )),
        }
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
        append_session_event_in_transaction(&self.connection, session_id, event, now)
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

    /// Loads one database-wide ordered batch of session events after a cursor.
    ///
    /// Event IDs come from SQLite's shared `session_events` row ID, so this
    /// cursor safely merges activity from API and CLI session runners. An
    /// optional typed kind keeps narrow consumers such as the desktop tray
    /// from receiving high-volume assistant delta events.
    pub fn load_global_session_events_after(
        &self,
        after_event_id: Option<i64>,
        kind: Option<SessionEventKind>,
    ) -> Result<Vec<SessionEventRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, session_id, payload, created_at
                FROM session_events
                WHERE id > ?1
                  AND (?2 IS NULL OR event_type = ?2)
                ORDER BY id ASC
                LIMIT ?3
                ",
            )
            .context("failed to prepare aggregate runtime event replay")?;

        statement
            .query_map(
                params![
                    after_event_id.unwrap_or(0),
                    kind.map(SessionEventKind::as_str),
                    GLOBAL_SESSION_EVENT_BATCH_SIZE,
                ],
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
            .context("failed to replay aggregate runtime events")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode aggregate runtime events")
    }

    /// Loads the latest database-wide event cursor, optionally for one kind.
    pub fn latest_global_session_event_id(
        &self,
        kind: Option<SessionEventKind>,
    ) -> Result<Option<i64>> {
        self.connection
            .query_row(
                "
                SELECT MAX(id)
                FROM session_events
                WHERE ?1 IS NULL OR event_type = ?1
                ",
                params![kind.map(SessionEventKind::as_str)],
                |row| row.get(0),
            )
            .context("failed to load latest aggregate runtime event cursor")
    }

    /// Loads the latest persisted event cursor for one session.
    ///
    /// The cursor is a snapshot boundary for clients that already hydrated a
    /// session. A client can subscribe after this ID to receive only events
    /// that happened after its snapshot instead of replaying the session's
    /// entire historical event log as new UI activity.
    pub fn latest_session_event_id(&self, session_id: &SessionId) -> Result<Option<i64>> {
        self.ensure_session_exists(session_id)?;

        self.connection
            .query_row(
                "SELECT MAX(id) FROM session_events WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| row.get(0),
            )
            .context("failed to load latest runtime event cursor")
    }

    /// Counts every persisted message on a session's current root-to-head path.
    ///
    /// The count includes messages inherited from the session's branch point.
    /// Sessions are pointers into one conversation tree, so this is the total
    /// model-visible path length rather than only the messages appended after
    /// `start_head_message_id`.
    pub fn session_node_count(&self, session: &Session) -> Result<usize> {
        let Some(head_message_id) = session.current_head_message_id.as_ref() else {
            return Ok(0);
        };

        Ok(self
            .load_path_to_message_rows(&session.conversation_id, head_message_id)?
            .len())
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

/// Appends one session event through an existing connection or transaction.
fn append_session_event_in_transaction(
    connection: &Connection,
    session_id: &SessionId,
    event: SessionEvent,
    now: i64,
) -> Result<SessionEventRecord> {
    let event_type = event.event_name();
    let payload = serde_json::to_string(&event).context("failed to encode runtime event")?;
    connection
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
    let id = connection.last_insert_rowid();

    Ok(SessionEventRecord {
        id,
        session_id: session_id.clone(),
        event,
        created_at: now,
    })
}

fn sessions_at_head(
    connection: &Connection,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<Vec<Session>> {
    let mut statement = connection
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
              AND (
                  current_head_message_id = ?2
                  OR (?2 IS NULL AND current_head_message_id IS NULL)
              )
            ORDER BY created_at, id
            ",
        )
        .context("failed to prepare session head resolution")?;

    statement
        .query_map(
            params![
                conversation_id.as_str(),
                head_message_id.map(MessageId::as_str)
            ],
            session_from_row,
        )
        .context("failed to resolve sessions at conversation head")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to decode session head resolution")
}

fn session_resolution_from_matches(sessions: Vec<Session>) -> SessionResolution {
    match sessions.len() {
        0 => SessionResolution::NoSessionAtHead,
        1 => SessionResolution::Existing(sessions.into_iter().next().expect("one session")),
        _ => SessionResolution::Ambiguous(sessions),
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
