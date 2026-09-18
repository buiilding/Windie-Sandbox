//! PostgreSQL persistence for the account-owned hosted conversation service.
//!
//! This intentionally remains a focused store instead of turning the local
//! SQLite `Store` into a broad database abstraction. The tree invariants are
//! shared; account ownership, revisions, idempotency, and event cursors are
//! hosted-server concerns.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgListener, types::Json};
use uuid::Uuid;

use super::account::HostedAccount;
use crate::{
    conversation::{
        ConversationTree, ConversationTreeError, ConversationTreeNode, ImageAssetId, ImagePart,
        Message, MessagePart, Role,
    },
    llm::ReasoningRequest,
    session::{
        ClaimedSession, IdleWakeupInterval, Session, SessionEvent, SessionEventRecord,
        SessionExecutionClaim, SessionExecutionClaimId, SessionExecutionOwner,
        SessionExecutionStart, SessionId, SessionInputId, SessionResolution, SessionStatus,
        can_start, resolve_sessions_at_head,
    },
};

const INITIAL_MIGRATION: &str = "0001_account_conversations";
const SESSIONS_MIGRATION: &str = "0002_sessions";
const SESSION_EVENT_NOTIFICATION_CHANNEL: &str = "windie_session_events";

/// PostgreSQL boundary for hosted Windie state.
#[derive(Clone)]
pub struct HostedStore {
    pool: PgPool,
}

/// Stable error categories exposed by hosted HTTP handlers.
#[derive(Debug, thiserror::Error)]
pub enum HostedStoreError {
    #[error("the requested conversation or message does not exist")]
    NotFound,
    #[error("conversation revision is stale (expected {expected}, current {current})")]
    StaleRevision { expected: i64, current: i64 },
    #[error("idempotency key is required and must be at most 255 characters")]
    InvalidIdempotencyKey,
    #[error("message text or parts are required")]
    EmptyMessage,
    #[error("role: tool messages are created only by hosted execution")]
    ToolMessageNotAllowed,
    #[error("message role must be system, user, or assistant")]
    InvalidRole,
    #[error("a message parent must belong to the same conversation")]
    InvalidParent,
    #[error("the selected head is not part of this conversation")]
    InvalidHead,
    #[error("message text must not be empty")]
    EmptyText,
    #[error("conversation model must not be empty")]
    EmptyModel,
    #[error("stored conversation tree is invalid")]
    InvalidTree,
    #[error("the requested session does not exist")]
    SessionNotFound,
    #[error("multiple sessions exist at the requested conversation head")]
    AmbiguousSession,
    #[error("session state conflicts with the requested operation")]
    SessionConflict,
    #[error("stored session data is invalid")]
    InvalidSession,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl From<ConversationTreeError> for HostedStoreError {
    fn from(error: ConversationTreeError) -> Self {
        match error {
            ConversationTreeError::MessageNotFound(_) => Self::NotFound,
            ConversationTreeError::ParentNotFound(_) => Self::InvalidParent,
            ConversationTreeError::InvalidTree(_)
            | ConversationTreeError::EmptyDelete
            | ConversationTreeError::DeletedSpliceParent => Self::InvalidTree,
        }
    }
}

/// Minimal conversation row used in list and mutation responses.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct HostedConversationSummary {
    pub(crate) id: String,
    pub(crate) title: Option<String>,
    pub(crate) model: String,
    pub(crate) revision: i64,
    pub(crate) message_count: i64,
}

/// Full canonical conversation graph returned by the hosted read endpoint.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct HostedConversation {
    #[serde(flatten)]
    pub(crate) summary: HostedConversationSummary,
    pub(crate) messages: Vec<HostedMessage>,
    pub(crate) selected_path: Option<Vec<String>>,
}

/// One parent-linked persisted message.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct HostedMessage {
    pub(crate) id: String,
    pub(crate) parent_message_id: Option<String>,
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) parts: Vec<HostedMessagePart>,
}

/// Read-safe projection of one stored message part.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum HostedMessagePart {
    Text {
        text: String,
    },
    Image {
        mime_type: String,
        byte_count: usize,
    },
}

/// Input accepted when appending a hosted message. File paths are excluded:
/// a browser cannot ask the server to read arbitrary server-local files.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum HostedMessagePartInput {
    Text { text: String },
    ImageData { mime_type: String, data: String },
}

/// Durable account change event used for catch-up and live SSE polling.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct HostedChangeEvent {
    pub(crate) id: i64,
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) conversation_revision: Option<i64>,
    pub(crate) payload: Value,
}

/// Stored result returned when an idempotent mutation is retried.
#[derive(Debug, Clone)]
pub(crate) struct MutationResponse {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

/// All PostgreSQL inputs for one account-owned message append.
///
/// This is a persistence request, not a browser request: its account is
/// supplied separately by authenticated server state, and it contains no HTTP
/// headers or transport behavior.
pub(crate) struct HostedAppendMutation<'a> {
    pub(crate) conversation_id: &'a str,
    pub(crate) expected_revision: i64,
    pub(crate) parent_message_id: Option<&'a str>,
    pub(crate) role: &'a str,
    pub(crate) parts: &'a [HostedMessagePartInput],
    pub(crate) idempotency_key: &'a str,
}

impl HostedStore {
    /// Opens a private PostgreSQL connection pool. Migration is kept separate
    /// so callers can make database changes explicit at process startup.
    pub async fn connect(database_url: &str) -> Result<Self, HostedStoreError> {
        Ok(Self {
            pool: PgPool::connect(database_url).await?,
        })
    }

    /// Applies the versioned hosted schema before serving requests.
    pub async fn migrate(&self) -> Result<(), HostedStoreError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS hosted_schema_migrations (version TEXT PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
        )
        .execute(&self.pool)
        .await?;
        for (version, migration) in [
            (
                INITIAL_MIGRATION,
                include_str!("../../migrations/hosted/0001_account_conversations.sql"),
            ),
            (
                SESSIONS_MIGRATION,
                include_str!("../../migrations/hosted/0002_sessions.sql"),
            ),
        ] {
            let applied = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM hosted_schema_migrations WHERE version = $1)",
            )
            .bind(version)
            .fetch_one(&self.pool)
            .await?;
            if !applied {
                let mut transaction = self.pool.begin().await?;
                sqlx::raw_sql(migration).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO hosted_schema_migrations (version) VALUES ($1)")
                    .bind(version)
                    .execute(&mut *transaction)
                    .await?;
                transaction.commit().await?;
            }
        }
        Ok(())
    }

    /// Finds or atomically creates the Windie account for a verified Supabase
    /// subject. The subject is never accepted from a client request body.
    pub(crate) async fn resolve_account(
        &self,
        auth_subject: &str,
    ) -> Result<HostedAccount, HostedStoreError> {
        let id = Uuid::new_v4().to_string();
        let row = sqlx::query(
            "INSERT INTO accounts (id, auth_subject) VALUES ($1, $2) \
             ON CONFLICT (auth_subject) DO UPDATE SET auth_subject = EXCLUDED.auth_subject \
             RETURNING id, auth_subject",
        )
        .bind(id)
        .bind(auth_subject)
        .fetch_one(&self.pool)
        .await?;
        Ok(HostedAccount {
            id: row.get("id"),
            auth_subject: row.get("auth_subject"),
        })
    }

    /// Lists only conversations owned by the authenticated account, alongside
    /// the durable event cursor needed to begin synchronization.
    pub(crate) async fn list_conversations(
        &self,
        account: &HostedAccount,
    ) -> Result<(Vec<HostedConversationSummary>, i64), HostedStoreError> {
        let rows = sqlx::query(
            "SELECT c.id, c.title, c.model, c.revision, COUNT(m.id) AS message_count \
             FROM conversations c LEFT JOIN messages m ON m.conversation_id = c.id \
             WHERE c.account_id = $1 GROUP BY c.id \
             ORDER BY c.updated_at DESC, c.id DESC",
        )
        .bind(&account.id)
        .fetch_all(&self.pool)
        .await?;
        let conversations = rows.iter().map(summary_from_row).collect();
        let cursor = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MAX(id) FROM account_change_events WHERE account_id = $1",
        )
        .bind(&account.id)
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0);
        Ok((conversations, cursor))
    }

    /// Loads one account-owned conversation and, when requested, validates and
    /// derives the root-to-selected-head path from the canonical tree.
    pub(crate) async fn conversation(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        selected_head: Option<&str>,
    ) -> Result<HostedConversation, HostedStoreError> {
        let summary = self.summary(account, conversation_id).await?;
        let messages = self.messages(conversation_id).await?;
        let selected_path = selected_head
            .map(|head| selected_path(&messages, head))
            .transpose()?;
        Ok(HostedConversation {
            summary,
            messages,
            selected_path,
        })
    }

    /// Creates an account-owned conversation with a durable account event.
    pub(crate) async fn create_conversation(
        &self,
        account: &HostedAccount,
        model: &str,
        idempotency_key: &str,
    ) -> Result<MutationResponse, HostedStoreError> {
        if model.trim().is_empty() {
            return Err(HostedStoreError::EmptyModel);
        }
        let mut transaction = self.pool.begin().await?;
        if let Some(response) =
            reserve_idempotency(&mut transaction, account, idempotency_key).await?
        {
            transaction.rollback().await?;
            return Ok(response);
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO conversations (id, account_id, model) VALUES ($1, $2, $3)")
            .bind(&id)
            .bind(&account.id)
            .bind(model.trim())
            .execute(&mut *transaction)
            .await?;
        let summary = summary_in_transaction(&mut transaction, account, &id).await?;
        let event = append_event(
            &mut transaction,
            account,
            "conversation.created",
            Some(&id),
            Some(0),
            json!({"conversation_id": id, "revision": 0}),
        )
        .await?;
        let body = json!({"conversation": summary, "event_cursor": event.id});
        let response = MutationResponse { status: 201, body };
        save_idempotency(&mut transaction, account, idempotency_key, &response).await?;
        transaction.commit().await?;
        Ok(response)
    }

    /// Appends a parent-linked non-tool message under the caller-selected head.
    pub(crate) async fn append_message(
        &self,
        account: &HostedAccount,
        mutation: HostedAppendMutation<'_>,
    ) -> Result<MutationResponse, HostedStoreError> {
        let HostedAppendMutation {
            conversation_id,
            expected_revision,
            parent_message_id,
            role,
            parts,
            idempotency_key,
        } = mutation;
        if role == "tool" {
            return Err(HostedStoreError::ToolMessageNotAllowed);
        }
        if !matches!(role, "system" | "user" | "assistant") {
            return Err(HostedStoreError::InvalidRole);
        }
        let prepared = prepare_parts(parts)?;
        let mut transaction = self.pool.begin().await?;
        // Reserve first. A timed-out retry carries the old revision, so its
        // original saved response must win before stale-revision checking.
        if let Some(response) =
            reserve_idempotency(&mut transaction, account, idempotency_key).await?
        {
            transaction.rollback().await?;
            return Ok(response);
        }
        lock_conversation(
            &mut transaction,
            account,
            conversation_id,
            expected_revision,
        )
        .await?;
        conversation_tree_in_transaction(&mut transaction, conversation_id)
            .await?
            .validate_append_parent(parent_message_id)?;
        let message_id = Uuid::new_v4().to_string();
        let content = prepared_content(&prepared);
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, parent_message_id, role, content) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&message_id)
        .bind(conversation_id)
        .bind(parent_message_id)
        .bind(role)
        .bind(&content)
        .execute(&mut *transaction)
        .await?;
        insert_parts(&mut transaction, &message_id, &prepared).await?;
        let revision = bump_revision(&mut transaction, conversation_id).await?;
        let event = append_event(
            &mut transaction,
            account,
            "conversation.message_added",
            Some(conversation_id),
            Some(revision),
            json!({"conversation_id": conversation_id, "message_id": message_id, "revision": revision}),
        )
        .await?;
        let body =
            json!({"message_id": message_id, "revision": revision, "event_cursor": event.id});
        let response = MutationResponse { status: 201, body };
        save_idempotency(&mut transaction, account, idempotency_key, &response).await?;
        transaction.commit().await?;
        Ok(response)
    }

    /// Replaces a message's visible content with one text part.
    pub(crate) async fn update_message(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        message_id: &str,
        expected_revision: i64,
        text: &str,
        idempotency_key: &str,
    ) -> Result<MutationResponse, HostedStoreError> {
        if text.trim().is_empty() {
            return Err(HostedStoreError::EmptyText);
        }
        let mut transaction = self.pool.begin().await?;
        if let Some(response) =
            reserve_idempotency(&mut transaction, account, idempotency_key).await?
        {
            transaction.rollback().await?;
            return Ok(response);
        }
        lock_conversation(
            &mut transaction,
            account,
            conversation_id,
            expected_revision,
        )
        .await?;
        conversation_tree_in_transaction(&mut transaction, conversation_id)
            .await?
            .require_message(message_id)?;
        let updated =
            sqlx::query("UPDATE messages SET content = $1 WHERE id = $2 AND conversation_id = $3")
                .bind(text)
                .bind(message_id)
                .bind(conversation_id)
                .execute(&mut *transaction)
                .await?
                .rows_affected();
        if updated != 1 {
            return Err(HostedStoreError::NotFound);
        }
        sqlx::query("DELETE FROM message_parts WHERE message_id = $1")
            .bind(message_id)
            .execute(&mut *transaction)
            .await?;
        insert_parts(
            &mut transaction,
            message_id,
            &[PreparedPart::Text(text.to_string())],
        )
        .await?;
        let revision = bump_revision(&mut transaction, conversation_id).await?;
        let event = append_event(
            &mut transaction,
            account,
            "conversation.message_updated",
            Some(conversation_id),
            Some(revision),
            json!({"conversation_id": conversation_id, "message_id": message_id, "revision": revision}),
        )
        .await?;
        let body =
            json!({"message_id": message_id, "revision": revision, "event_cursor": event.id});
        let response = MutationResponse { status: 200, body };
        save_idempotency(&mut transaction, account, idempotency_key, &response).await?;
        transaction.commit().await?;
        Ok(response)
    }

    /// Removes one ordinary message and splices its direct descendants to its
    /// parent, matching the local tree behavior for non-tool message groups.
    pub(crate) async fn remove_message(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        message_id: &str,
        expected_revision: i64,
        idempotency_key: &str,
    ) -> Result<MutationResponse, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(response) =
            reserve_idempotency(&mut transaction, account, idempotency_key).await?
        {
            transaction.rollback().await?;
            return Ok(response);
        }
        lock_conversation(
            &mut transaction,
            account,
            conversation_id,
            expected_revision,
        )
        .await?;
        let plan = conversation_tree_in_transaction(&mut transaction, conversation_id)
            .await?
            .plan_remove_message(message_id)?;
        for promoted_child_id in &plan.promoted_child_ids {
            sqlx::query(
                "UPDATE messages SET parent_message_id = $1 WHERE conversation_id = $2 AND id = $3",
            )
            .bind(&plan.splice_parent_message_id)
            .bind(conversation_id)
            .bind(promoted_child_id)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("DELETE FROM messages WHERE conversation_id = $1 AND id = ANY($2)")
            .bind(conversation_id)
            .bind(plan.deleted_message_ids.into_iter().collect::<Vec<_>>())
            .execute(&mut *transaction)
            .await?;
        let revision = bump_revision(&mut transaction, conversation_id).await?;
        let event = append_event(
            &mut transaction,
            account,
            "conversation.message_removed",
            Some(conversation_id),
            Some(revision),
            json!({"conversation_id": conversation_id, "message_id": message_id, "revision": revision}),
        )
        .await?;
        let body = json!({"deleted": true, "revision": revision, "event_cursor": event.id});
        let response = MutationResponse { status: 200, body };
        save_idempotency(&mut transaction, account, idempotency_key, &response).await?;
        transaction.commit().await?;
        Ok(response)
    }

    /// Deletes descendants below a checkpoint without deleting the checkpoint.
    pub(crate) async fn truncate_after_message(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        checkpoint_id: &str,
        expected_revision: i64,
        idempotency_key: &str,
    ) -> Result<MutationResponse, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(response) =
            reserve_idempotency(&mut transaction, account, idempotency_key).await?
        {
            transaction.rollback().await?;
            return Ok(response);
        }
        lock_conversation(
            &mut transaction,
            account,
            conversation_id,
            expected_revision,
        )
        .await?;
        let plan = conversation_tree_in_transaction(&mut transaction, conversation_id)
            .await?
            .plan_truncate_after(checkpoint_id)?;
        if !plan.deleted_message_ids.is_empty() {
            sqlx::query("DELETE FROM messages WHERE conversation_id = $1 AND id = ANY($2)")
                .bind(conversation_id)
                .bind(plan.deleted_message_ids.into_iter().collect::<Vec<_>>())
                .execute(&mut *transaction)
                .await?;
        }
        let revision = bump_revision(&mut transaction, conversation_id).await?;
        let event = append_event(
            &mut transaction,
            account,
            "conversation.truncated",
            Some(conversation_id),
            Some(revision),
            json!({"conversation_id": conversation_id, "message_id": checkpoint_id, "revision": revision}),
        )
        .await?;
        let body =
            json!({"message_id": checkpoint_id, "revision": revision, "event_cursor": event.id});
        let response = MutationResponse { status: 200, body };
        save_idempotency(&mut transaction, account, idempotency_key, &response).await?;
        transaction.commit().await?;
        Ok(response)
    }

    /// Forks the root-to-head path into a new account-owned conversation.
    pub(crate) async fn fork_conversation(
        &self,
        account: &HostedAccount,
        source_conversation_id: &str,
        head_message_id: &str,
        expected_revision: i64,
        idempotency_key: &str,
    ) -> Result<MutationResponse, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(response) =
            reserve_idempotency(&mut transaction, account, idempotency_key).await?
        {
            transaction.rollback().await?;
            return Ok(response);
        }
        lock_conversation(
            &mut transaction,
            account,
            source_conversation_id,
            expected_revision,
        )
        .await?;
        let source =
            summary_in_transaction(&mut transaction, account, source_conversation_id).await?;
        let messages = messages_in_transaction(&mut transaction, source_conversation_id).await?;
        let path = conversation_tree_from_messages(&messages)?.selected_path(head_message_id)?;
        let by_id = messages
            .into_iter()
            .map(|message| (message.id.clone(), message))
            .collect::<HashMap<_, _>>();
        let fork_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO conversations (id, account_id, title, model) VALUES ($1, $2, $3, $4)",
        )
        .bind(&fork_id)
        .bind(&account.id)
        .bind(source.title)
        .bind(source.model)
        .execute(&mut *transaction)
        .await?;
        let mut copied_ids = HashMap::new();
        for old_id in &path {
            let message = by_id.get(old_id).ok_or(HostedStoreError::InvalidTree)?;
            let new_id = Uuid::new_v4().to_string();
            let new_parent = message
                .parent_message_id
                .as_ref()
                .and_then(|parent| copied_ids.get(parent));
            sqlx::query(
                "INSERT INTO messages (id, conversation_id, parent_message_id, role, content) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&new_id)
            .bind(&fork_id)
            .bind(new_parent)
            .bind(&message.role)
            .bind(&message.content)
            .execute(&mut *transaction)
            .await?;
            copy_message_parts(&mut transaction, old_id, &new_id).await?;
            copied_ids.insert(old_id.clone(), new_id);
        }
        let summary = summary_in_transaction(&mut transaction, account, &fork_id).await?;
        let event = append_event(
            &mut transaction,
            account,
            "conversation.forked",
            Some(&fork_id),
            Some(0),
            json!({"source_conversation_id": source_conversation_id, "conversation_id": fork_id, "revision": 0}),
        )
        .await?;
        let body = json!({"conversation": summary, "event_cursor": event.id});
        let response = MutationResponse { status: 201, body };
        save_idempotency(&mut transaction, account, idempotency_key, &response).await?;
        transaction.commit().await?;
        Ok(response)
    }

    /// Reads durable account change events strictly after a client cursor.
    pub(crate) async fn events_after(
        &self,
        account: &HostedAccount,
        after: i64,
    ) -> Result<Vec<HostedChangeEvent>, HostedStoreError> {
        let rows = sqlx::query(
            "SELECT id, event_type, conversation_id, conversation_revision, payload \
             FROM account_change_events WHERE account_id = $1 AND id > $2 ORDER BY id ASC",
        )
        .bind(&account.id)
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(event_from_row).collect())
    }

    /// Resolves a canonical conversation head to one hosted session or creates
    /// the first branch session while the conversation row is locked.
    pub(crate) async fn resolve_or_create_session(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        head_message_id: Option<&str>,
        reasoning: Option<ReasoningRequest>,
    ) -> Result<Session, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let conversation = sqlx::query(
            "SELECT model FROM conversations WHERE id = $1 AND account_id = $2 FOR UPDATE",
        )
        .bind(conversation_id)
        .bind(&account.id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(HostedStoreError::NotFound)?;
        if let Some(head) = head_message_id {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id = $1 AND conversation_id = $2)",
            )
            .bind(head)
            .bind(conversation_id)
            .fetch_one(&mut *transaction)
            .await?;
            if !exists {
                return Err(HostedStoreError::InvalidHead);
            }
        }

        let matches =
            sessions_at_head_in_transaction(&mut transaction, conversation_id, head_message_id)
                .await?;
        match resolve_sessions_at_head(matches) {
            SessionResolution::Existing(session) => {
                transaction.commit().await?;
                Ok(*session)
            }
            SessionResolution::Ambiguous(_) => Err(HostedStoreError::AmbiguousSession),
            SessionResolution::NoSessionAtHead => {
                let now = now_millis();
                let session_id = SessionId::fresh();
                let model: String = conversation.get("model");
                sqlx::query(
                    "INSERT INTO sessions (id, account_id, conversation_id, start_head_message_id, current_head_message_id, status, model, reasoning, last_user_activity_at) \
                     VALUES ($1, $2, $3, $4, $4, 'ready', $5, $6, $7)",
                )
                .bind(session_id.as_str())
                .bind(&account.id)
                .bind(conversation_id)
                .bind(head_message_id)
                .bind(model)
                .bind(reasoning.map(Json))
                .bind(now)
                .execute(&mut *transaction)
                .await?;
                let session =
                    load_session_in_transaction(&mut transaction, &account.id, &session_id).await?;
                transaction.commit().await?;
                Ok(session)
            }
        }
    }

    /// Loads an account-owned hosted session. The account filter is mandatory
    /// even though the session also points to an account-owned conversation.
    pub(crate) async fn session(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> Result<Session, HostedStoreError> {
        let row = sqlx::query(&session_select_sql("s.id = $1 AND s.account_id = $2"))
            .bind(session_id.as_str())
            .bind(&account.id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(HostedStoreError::SessionNotFound)?;
        session_from_pg_row(&row)
    }

    /// Resolves the durable branch state without creating a session.
    pub(crate) async fn resolve_session_at_head(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        head_message_id: Option<&str>,
    ) -> Result<SessionResolution, HostedStoreError> {
        let conversation_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = $1 AND account_id = $2)",
        )
        .bind(conversation_id)
        .bind(&account.id)
        .fetch_one(&self.pool)
        .await?;
        if !conversation_exists {
            return Err(HostedStoreError::NotFound);
        }
        if let Some(head) = head_message_id {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id = $1 AND conversation_id = $2)",
            )
            .bind(head)
            .bind(conversation_id)
            .fetch_one(&self.pool)
            .await?;
            if !exists {
                return Err(HostedStoreError::InvalidHead);
            }
        }
        let rows = sqlx::query(&format!(
            "{} ORDER BY s.created_at, s.id",
            session_select_sql("s.account_id = $1 AND s.conversation_id = $2 AND (s.current_head_message_id = $3 OR ($3 IS NULL AND s.current_head_message_id IS NULL))")
        ))
        .bind(&account.id)
        .bind(conversation_id)
        .bind(head_message_id)
        .fetch_all(&self.pool)
        .await?;
        let sessions = rows
            .iter()
            .map(session_from_pg_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(resolve_sessions_at_head(sessions))
    }

    /// Claims a session exclusively. Every later owned write validates the
    /// generated fencing token, so an old worker cannot write through a newer
    /// execution after restart or failover.
    pub(crate) async fn claim_session_execution(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        start: SessionExecutionStart,
    ) -> Result<ClaimedSession, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let session = load_session_for_update(&mut transaction, &account.id, session_id).await?;
        if session_execution_claim_in_transaction(&mut transaction, session_id)
            .await?
            .is_some()
            || !can_start(&session, &start, now_millis())
        {
            return Err(HostedStoreError::SessionConflict);
        }
        let claim = SessionExecutionClaim::fresh(SessionExecutionOwner::HostedServer);
        sqlx::query(
            "INSERT INTO session_execution_claims (id, session_id, owner, status) VALUES ($1, $2, $3, 'active')",
        )
        .bind(claim.id.as_str())
        .bind(session_id.as_str())
        .bind(claim.owner.as_storage())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE sessions SET status = 'running', error = NULL, execution_owner = $1, current_claim_id = $2, updated_at = now() WHERE id = $3",
        )
        .bind(claim.owner.as_storage())
        .bind(claim.id.as_str())
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let session =
            load_session_in_transaction(&mut transaction, &account.id, session_id).await?;
        transaction.commit().await?;
        Ok(ClaimedSession { session, claim })
    }

    /// Appends text input under a claimed session head and advances that head.
    pub(crate) async fn append_session_input(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        parts: &[HostedMessagePartInput],
    ) -> Result<(), HostedStoreError> {
        let parts = prepare_parts(parts)?;
        let content = prepared_content(&parts);
        let mut transaction = self.pool.begin().await?;
        let session =
            claimed_session_for_update(&mut transaction, &account.id, session_id, claim).await?;
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, parent_message_id, role, content) VALUES ($1, $2, $3, 'user', $4)",
        )
        .bind(&message_id)
        .bind(session.conversation_id.as_str())
        .bind(session.current_head_message_id.as_ref().map(|id| id.as_str()))
        .bind(&content)
        .execute(&mut *transaction)
        .await?;
        insert_parts(&mut transaction, &message_id, &parts).await?;
        sqlx::query(
            "UPDATE sessions SET current_head_message_id = $1, last_user_activity_at = $2, updated_at = now() WHERE id = $3 AND current_claim_id = $4",
        )
        .bind(&message_id)
        .bind(now_millis())
        .bind(session_id.as_str())
        .bind(claim.id.as_str())
        .execute(&mut *transaction)
        .await?;
        let revision = bump_revision(&mut transaction, session.conversation_id.as_str()).await?;
        append_event(
            &mut transaction,
            account,
            "conversation.changed",
            Some(session.conversation_id.as_str()),
            Some(revision),
            json!({"conversation_id": session.conversation_id.as_str(), "message_id": message_id, "revision": revision}),
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Enqueues user input with a per-session monotonically increasing FIFO
    /// position while a session is running.
    pub(crate) async fn enqueue_session_input(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        parts: &[HostedMessagePartInput],
    ) -> Result<(SessionInputId, usize, SessionEventRecord), HostedStoreError> {
        let prepared = prepare_parts(parts)?;
        let content = prepared_content(&prepared);
        let mut transaction = self.pool.begin().await?;
        let session = load_session_for_update(&mut transaction, &account.id, session_id).await?;
        if session.status != SessionStatus::Running {
            return Err(HostedStoreError::SessionConflict);
        }
        let position = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM session_inputs WHERE session_id = $1",
        )
        .bind(session_id.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let input_id = SessionInputId::fresh();
        sqlx::query(
            "INSERT INTO session_inputs (id, session_id, position, content, parts) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(input_id.as_str())
        .bind(session_id.as_str())
        .bind(position)
        .bind(&content)
        .bind(Json(parts))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE sessions SET last_user_activity_at = $1, updated_at = now() WHERE id = $2",
        )
        .bind(now_millis())
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let depth = (position + 1) as usize;
        let record = append_hosted_session_event(
            &mut transaction,
            session_id,
            SessionEvent::InputQueued {
                input_id: input_id.as_str().to_string(),
                queue_depth: depth,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok((input_id, depth, record))
    }

    /// Atomically turns the oldest queued input into a canonical user message
    /// after its worker has claimed the session.
    pub(crate) async fn materialize_next_session_input(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
    ) -> Result<Option<SessionEventRecord>, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let session =
            claimed_session_for_update(&mut transaction, &account.id, session_id, claim).await?;
        let queued = sqlx::query(
            "SELECT id, content, parts FROM session_inputs WHERE session_id = $1 ORDER BY position ASC FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .bind(session_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(queued) = queued else {
            transaction.commit().await?;
            return Ok(None);
        };
        let input_id: String = queued.get("id");
        let content: String = queued.get("content");
        let parts = queued
            .get::<Json<Vec<HostedMessagePartInput>>, _>("parts")
            .0;
        let parts = prepare_parts(&parts)?;
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, parent_message_id, role, content) VALUES ($1, $2, $3, 'user', $4)",
        )
        .bind(&message_id)
        .bind(session.conversation_id.as_str())
        .bind(session.current_head_message_id.as_ref().map(|id| id.as_str()))
        .bind(&content)
        .execute(&mut *transaction)
        .await?;
        insert_parts(&mut transaction, &message_id, &parts).await?;
        sqlx::query("DELETE FROM session_inputs WHERE id = $1")
            .bind(&input_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "UPDATE sessions SET current_head_message_id = $1, updated_at = now() WHERE id = $2 AND current_claim_id = $3",
        )
        .bind(&message_id)
        .bind(session_id.as_str())
        .bind(claim.id.as_str())
        .execute(&mut *transaction)
        .await?;
        let revision = bump_revision(&mut transaction, session.conversation_id.as_str()).await?;
        let record = append_hosted_session_event(
            &mut transaction,
            session_id,
            SessionEvent::InputStarted {
                input_id,
                message_id: message_id.clone(),
            },
        )
        .await?;
        append_event(
            &mut transaction,
            account,
            "conversation.changed",
            Some(session.conversation_id.as_str()),
            Some(revision),
            json!({"conversation_id": session.conversation_id.as_str(), "message_id": message_id, "revision": revision}),
        )
        .await?;
        transaction.commit().await?;
        Ok(Some(record))
    }

    /// Persists one replayable stream event only while the provided execution
    /// claim is still current.
    pub(crate) async fn append_session_execution_event(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        event: SessionEvent,
    ) -> Result<SessionEventRecord, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        claimed_session_for_update(&mut transaction, &account.id, session_id, claim).await?;
        let record = append_hosted_session_event(&mut transaction, session_id, event).await?;
        transaction.commit().await?;
        Ok(record)
    }

    /// Saves the completed assistant message, advances the session head, and
    /// releases the claim in one transaction.
    pub(crate) async fn complete_session_with_assistant(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        content: &str,
    ) -> Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let session =
            claimed_session_for_update(&mut transaction, &account.id, session_id, claim).await?;
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, parent_message_id, role, content) VALUES ($1, $2, $3, 'assistant', $4)",
        )
        .bind(&message_id)
        .bind(session.conversation_id.as_str())
        .bind(session.current_head_message_id.as_ref().map(|id| id.as_str()))
        .bind(content)
        .execute(&mut *transaction)
        .await?;
        if !content.is_empty() {
            insert_parts(
                &mut transaction,
                &message_id,
                &[PreparedPart::Text(content.to_string())],
            )
            .await?;
        }
        sqlx::query(
            "UPDATE sessions SET current_head_message_id = $1, status = 'completed', execution_owner = NULL, current_claim_id = NULL, updated_at = now() WHERE id = $2 AND current_claim_id = $3",
        )
        .bind(&message_id)
        .bind(session_id.as_str())
        .bind(claim.id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE session_execution_claims SET status = 'completed', released_at = now() WHERE id = $1 AND status = 'active'",
        )
        .bind(claim.id.as_str())
        .execute(&mut *transaction)
        .await?;
        let revision = bump_revision(&mut transaction, session.conversation_id.as_str()).await?;
        let saved = append_hosted_session_event(
            &mut transaction,
            session_id,
            SessionEvent::AssistantMessageSaved {
                message_id: message_id.clone(),
            },
        )
        .await?;
        let completed = append_hosted_session_event(
            &mut transaction,
            session_id,
            SessionEvent::Completed {
                message_id: Some(message_id.clone()),
            },
        )
        .await?;
        append_event(
            &mut transaction,
            account,
            "conversation.changed",
            Some(session.conversation_id.as_str()),
            Some(revision),
            json!({"conversation_id": session.conversation_id.as_str(), "message_id": message_id, "revision": revision}),
        )
        .await?;
        transaction.commit().await?;
        Ok(vec![saved, completed])
    }

    /// Records a failed claim. A failure is terminal and never silently
    /// replays after a hosted-server restart.
    pub(crate) async fn fail_claimed_session(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
        error: &str,
    ) -> Result<Option<SessionEventRecord>, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        match claimed_session_for_update(&mut transaction, &account.id, session_id, claim).await {
            Ok(_) => {}
            Err(HostedStoreError::SessionConflict) => {
                transaction.rollback().await?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        sqlx::query(
            "UPDATE sessions SET status = 'failed', error = $1, execution_owner = NULL, current_claim_id = NULL, updated_at = now() WHERE id = $2 AND current_claim_id = $3",
        )
        .bind(error)
        .bind(session_id.as_str())
        .bind(claim.id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE session_execution_claims SET status = 'failed', released_at = now() WHERE id = $1 AND status = 'active'",
        )
        .bind(claim.id.as_str())
        .execute(&mut *transaction)
        .await?;
        let record = append_hosted_session_event(
            &mut transaction,
            session_id,
            SessionEvent::Failed {
                error: error.to_string(),
                causes: vec![],
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(Some(record))
    }

    /// Cancels a hosted session and fences its current worker immediately.
    pub(crate) async fn stop_session(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> Result<(Session, SessionEventRecord), HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        load_session_for_update(&mut transaction, &account.id, session_id).await?;
        let claim = session_execution_claim_in_transaction(&mut transaction, session_id).await?;
        sqlx::query(
            "UPDATE sessions SET status = 'cancelled', execution_owner = NULL, current_claim_id = NULL, updated_at = now() WHERE id = $1",
        )
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if let Some(claim) = claim {
            sqlx::query(
                "UPDATE session_execution_claims SET status = 'cancelled', released_at = now() WHERE id = $1 AND status = 'active'",
            )
            .bind(claim.id.as_str())
            .execute(&mut *transaction)
            .await?;
        }
        let record =
            append_hosted_session_event(&mut transaction, session_id, SessionEvent::Cancelled)
                .await?;
        let session =
            load_session_in_transaction(&mut transaction, &account.id, session_id).await?;
        transaction.commit().await?;
        Ok((session, record))
    }

    /// Fails any hosted worker claim left running by a process crash before
    /// accepting new work. This deliberately does not replay unknown provider
    /// progress, which could duplicate an assistant response or charge.
    pub(crate) async fn recover_interrupted_hosted_sessions(
        &self,
    ) -> Result<u64, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, current_claim_id FROM sessions WHERE status = 'running' AND execution_owner = 'hosted_server' FOR UPDATE",
        )
        .fetch_all(&mut *transaction)
        .await?;
        for row in &rows {
            let session_id = SessionId::new(row.get::<String, _>("id"));
            let claim_id: Option<String> = row.get("current_claim_id");
            sqlx::query(
                "UPDATE sessions SET status = 'failed', error = 'hosted server restarted during execution', execution_owner = NULL, current_claim_id = NULL, updated_at = now() WHERE id = $1",
            )
            .bind(session_id.as_str())
            .execute(&mut *transaction)
            .await?;
            if let Some(claim_id) = claim_id {
                sqlx::query(
                    "UPDATE session_execution_claims SET status = 'failed', released_at = now() WHERE id = $1 AND status = 'active'",
                )
                .bind(claim_id)
                .execute(&mut *transaction)
                .await?;
            }
            append_hosted_session_event(
                &mut transaction,
                &session_id,
                SessionEvent::Failed {
                    error: "hosted server restarted during execution".to_string(),
                    causes: vec![],
                },
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(rows.len() as u64)
    }

    /// Reads durable session events strictly after one replay cursor.
    pub(crate) async fn session_events_after(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        after: i64,
    ) -> Result<Vec<SessionEventRecord>, HostedStoreError> {
        self.session(account, session_id).await?;
        let rows = sqlx::query(
            "SELECT id, event_type, payload, floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM session_events WHERE session_id = $1 AND id > $2 ORDER BY id ASC",
        )
        .bind(session_id.as_str())
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| session_event_from_pg_row(row, session_id))
            .collect()
    }

    /// Opens a PostgreSQL listener used only to wake other hosted-server
    /// processes after a durable session-event commit.
    pub(crate) async fn session_event_listener(&self) -> Result<PgListener, HostedStoreError> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen(SESSION_EVENT_NOTIFICATION_CHANNEL).await?;
        Ok(listener)
    }

    /// Loads one durable event by its database-wide cursor for an internal
    /// cross-instance notification. HTTP authorization still happens before a
    /// browser can subscribe to the session hub.
    pub(crate) async fn session_event_by_id(
        &self,
        event_id: i64,
    ) -> Result<Option<SessionEventRecord>, HostedStoreError> {
        let row = sqlx::query(
            "SELECT id, session_id, event_type, payload, floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM session_events WHERE id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let session_id = SessionId::new(row.get::<String, _>("session_id"));
            session_event_from_pg_row(&row, &session_id)
        })
        .transpose()
    }

    /// Returns the durable replay cursor immediately before a client opens a
    /// session event stream. Account ownership is checked first so the cursor
    /// never reveals whether another account has session activity.
    pub(crate) async fn session_event_cursor(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> Result<i64, HostedStoreError> {
        self.session(account, session_id).await?;
        let cursor = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(id), 0) FROM session_events WHERE session_id = $1",
        )
        .bind(session_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(cursor)
    }

    /// Returns canonical root-to-current-head model context for one still
    /// claimed execution. The hosted runtime deliberately passes no local MCP
    /// schemas, so model-requested tools cannot execute on the server.
    pub(crate) async fn model_messages_for_claim(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
    ) -> Result<(Session, Vec<Message>), HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let session =
            claimed_session_for_update(&mut transaction, &account.id, session_id, claim).await?;
        let model_messages = model_path_in_transaction(
            &mut transaction,
            session.conversation_id.as_str(),
            session
                .current_head_message_id
                .as_ref()
                .map(|id| id.as_str()),
        )
        .await?;
        transaction.commit().await?;
        Ok((session, model_messages))
    }

    /// Counts durable queued work for an account-owned session.
    pub(crate) async fn session_input_count(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> Result<usize, HostedStoreError> {
        self.session(account, session_id).await?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM session_inputs WHERE session_id = $1",
        )
        .bind(session_id.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(count as usize)
    }

    /// Persists one scheduled hosted wakeup. The browser supplies no account
    /// identity; the server proves the session belongs to its authenticated
    /// account before scheduling anything.
    pub(crate) async fn schedule_wakeup(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        trigger_type: &str,
        due_at_millis: i64,
    ) -> Result<String, HostedStoreError> {
        if trigger_type.trim().is_empty() || trigger_type.len() > 100 || due_at_millis < 0 {
            return Err(HostedStoreError::SessionConflict);
        }
        self.session(account, session_id).await?;
        let wakeup_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO wakeups (id, session_id, trigger_type, due_at) VALUES ($1, $2, $3, to_timestamp($4 / 1000.0))",
        )
        .bind(&wakeup_id)
        .bind(session_id.as_str())
        .bind(trigger_type)
        .bind(due_at_millis)
        .execute(&self.pool)
        .await?;
        Ok(wakeup_id)
    }

    /// Atomically takes one due wakeup and claims its session for the hosted
    /// worker. `SKIP LOCKED` lets multiple server instances share the schedule
    /// without processing the same wakeup twice.
    pub(crate) async fn claim_due_wakeup(
        &self,
    ) -> Result<Option<(HostedAccount, ClaimedSession, String)>, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT w.id AS wakeup_id, s.id AS session_id, a.id AS account_id, a.auth_subject \
             FROM wakeups w \
             JOIN sessions s ON s.id = w.session_id \
             JOIN accounts a ON a.id = s.account_id \
             WHERE w.status = 'pending' AND w.due_at <= now() \
             ORDER BY w.due_at, w.id FOR UPDATE OF w SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.commit().await?;
            return Ok(None);
        };
        let account = HostedAccount {
            id: row.get("account_id"),
            auth_subject: row.get("auth_subject"),
        };
        let session_id = SessionId::new(row.get::<String, _>("session_id"));
        let wakeup_id: String = row.get("wakeup_id");
        let session = load_session_for_update(&mut transaction, &account.id, &session_id).await?;
        if session_execution_claim_in_transaction(&mut transaction, &session_id)
            .await?
            .is_some()
            || !can_start(&session, &SessionExecutionStart::Runnable, now_millis())
        {
            transaction.commit().await?;
            return Ok(None);
        }
        let claim = SessionExecutionClaim::fresh(SessionExecutionOwner::HostedServer);
        sqlx::query(
            "INSERT INTO session_execution_claims (id, session_id, owner, status) VALUES ($1, $2, $3, 'active')",
        )
        .bind(claim.id.as_str())
        .bind(session_id.as_str())
        .bind(claim.owner.as_storage())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE sessions SET status = 'running', error = NULL, execution_owner = $1, current_claim_id = $2, updated_at = now() WHERE id = $3",
        )
        .bind(claim.owner.as_storage())
        .bind(claim.id.as_str())
        .bind(session_id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE wakeups SET status = 'claimed', claim_id = $1, claimed_at = now() WHERE id = $2 AND status = 'pending'",
        )
        .bind(claim.id.as_str())
        .bind(&wakeup_id)
        .execute(&mut *transaction)
        .await?;
        let session =
            load_session_in_transaction(&mut transaction, &account.id, &session_id).await?;
        transaction.commit().await?;
        Ok(Some((
            account,
            ClaimedSession { session, claim },
            wakeup_id,
        )))
    }

    /// Marks a claimed wakeup terminal after the owning worker exits.
    pub(crate) async fn finish_wakeup(
        &self,
        wakeup_id: &str,
        claim: &SessionExecutionClaim,
        completed: bool,
    ) -> Result<(), HostedStoreError> {
        sqlx::query(
            "UPDATE wakeups SET status = $1, completed_at = now() WHERE id = $2 AND claim_id = $3 AND status = 'claimed'",
        )
        .bind(if completed { "completed" } else { "cancelled" })
        .bind(wakeup_id)
        .bind(claim.id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn summary(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
    ) -> Result<HostedConversationSummary, HostedStoreError> {
        let row = sqlx::query(
            "SELECT c.id, c.title, c.model, c.revision, COUNT(m.id) AS message_count \
             FROM conversations c LEFT JOIN messages m ON m.conversation_id = c.id \
             WHERE c.account_id = $1 AND c.id = $2 GROUP BY c.id",
        )
        .bind(&account.id)
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(HostedStoreError::NotFound)?;
        Ok(summary_from_row(&row))
    }

    async fn messages(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<HostedMessage>, HostedStoreError> {
        let mut transaction = self.pool.begin().await?;
        let messages = messages_in_transaction(&mut transaction, conversation_id).await?;
        transaction.commit().await?;
        Ok(messages)
    }
}

/// Common selected fields for all hosted session reads.
fn session_select_sql(predicate: &str) -> String {
    format!(
        "SELECT s.id, s.conversation_id, s.start_head_message_id, s.current_head_message_id, s.status, s.model, s.reasoning, s.error, s.keep_awake, s.idle_wakeup_interval, s.last_user_activity_at, s.last_idle_wakeup_completed_at, floor(extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, floor(extract(epoch FROM s.updated_at) * 1000)::bigint AS updated_at FROM sessions s WHERE {predicate}"
    )
}

/// Decodes a PostgreSQL session row into the shared session domain type.
fn session_from_pg_row(row: &sqlx::postgres::PgRow) -> Result<Session, HostedStoreError> {
    let status_text: String = row.get("status");
    let interval_text: String = row.get("idle_wakeup_interval");
    Ok(Session {
        id: SessionId::new(row.get::<String, _>("id")),
        conversation_id: crate::conversation::ConversationId::new(
            row.get::<String, _>("conversation_id"),
        ),
        start_head_message_id: row
            .get::<Option<String>, _>("start_head_message_id")
            .map(crate::conversation::MessageId::new),
        current_head_message_id: row
            .get::<Option<String>, _>("current_head_message_id")
            .map(crate::conversation::MessageId::new),
        status: SessionStatus::from_storage(&status_text)
            .ok_or(HostedStoreError::InvalidSession)?,
        model: row.get("model"),
        reasoning: row
            .get::<Option<Json<ReasoningRequest>>, _>("reasoning")
            .map(|value| value.0),
        error: row.get("error"),
        keep_awake: row.get("keep_awake"),
        idle_wakeup_interval: IdleWakeupInterval::from_storage(&interval_text)
            .ok_or(HostedStoreError::InvalidSession)?,
        last_user_activity_at: row.get("last_user_activity_at"),
        last_idle_wakeup_completed_at: row.get("last_idle_wakeup_completed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Loads all sessions that end at `head_message_id` while the conversation
/// owner lock is held by the caller.
async fn sessions_at_head_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    conversation_id: &str,
    head_message_id: Option<&str>,
) -> Result<Vec<Session>, HostedStoreError> {
    let rows = sqlx::query(&format!(
        "{} ORDER BY s.created_at, s.id",
        session_select_sql("s.conversation_id = $1 AND (s.current_head_message_id = $2 OR ($2 IS NULL AND s.current_head_message_id IS NULL))")
    ))
    .bind(conversation_id)
    .bind(head_message_id)
    .fetch_all(&mut **transaction)
    .await?;
    rows.iter().map(session_from_pg_row).collect()
}

/// Loads an account-scoped session under a row lock.
async fn load_session_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    account_id: &str,
    session_id: &SessionId,
) -> Result<Session, HostedStoreError> {
    let row = sqlx::query(&format!(
        "{} FOR UPDATE",
        session_select_sql("s.id = $1 AND s.account_id = $2")
    ))
    .bind(session_id.as_str())
    .bind(account_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(HostedStoreError::SessionNotFound)?;
    session_from_pg_row(&row)
}

/// Loads a session when the surrounding caller already owns the transaction.
async fn load_session_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    account_id: &str,
    session_id: &SessionId,
) -> Result<Session, HostedStoreError> {
    let row = sqlx::query(&session_select_sql("s.id = $1 AND s.account_id = $2"))
        .bind(session_id.as_str())
        .bind(account_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(HostedStoreError::SessionNotFound)?;
    session_from_pg_row(&row)
}

/// Returns the active claim recorded on a locked session, if any.
async fn session_execution_claim_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: &SessionId,
) -> Result<Option<SessionExecutionClaim>, HostedStoreError> {
    let row = sqlx::query(
        "SELECT execution_owner, current_claim_id FROM sessions WHERE id = $1 FOR UPDATE",
    )
    .bind(session_id.as_str())
    .fetch_one(&mut **transaction)
    .await?;
    match (
        row.get::<Option<String>, _>("execution_owner"),
        row.get::<Option<String>, _>("current_claim_id"),
    ) {
        (None, None) => Ok(None),
        (Some(owner), Some(id)) => Ok(Some(SessionExecutionClaim {
            owner: SessionExecutionOwner::from_storage(&owner)
                .ok_or(HostedStoreError::InvalidSession)?,
            id: SessionExecutionClaimId::new(id),
        })),
        _ => Err(HostedStoreError::InvalidSession),
    }
}

/// Validates a runner's fencing token while keeping the session row locked.
async fn claimed_session_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    account_id: &str,
    session_id: &SessionId,
    claim: &SessionExecutionClaim,
) -> Result<Session, HostedStoreError> {
    let session = load_session_for_update(transaction, account_id, session_id).await?;
    let stored = session_execution_claim_in_transaction(transaction, session_id).await?;
    if session.status != SessionStatus::Running
        || !stored.is_some_and(|stored| {
            stored.owner == claim.owner && stored.id.as_str() == claim.id.as_str()
        })
    {
        return Err(HostedStoreError::SessionConflict);
    }
    Ok(session)
}

/// Appends an event in the same transaction as the state change it describes.
async fn append_hosted_session_event(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: &SessionId,
    event: SessionEvent,
) -> Result<SessionEventRecord, HostedStoreError> {
    let event_type = event.event_name();
    let row = sqlx::query(
        "INSERT INTO session_events (session_id, event_type, payload) VALUES ($1, $2, $3) RETURNING id, floor(extract(epoch FROM created_at) * 1000)::bigint AS created_at",
    )
    .bind(session_id.as_str())
    .bind(event_type)
    .bind(Json(&event))
    .fetch_one(&mut **transaction)
    .await?;
    let record = SessionEventRecord {
        id: row.get("id"),
        session_id: session_id.clone(),
        event,
        created_at: row.get("created_at"),
    };
    sqlx::query("SELECT pg_notify('windie_session_events', $1)")
        .bind(record.id.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(record)
}

/// Decodes one replayed event using the existing shared session event enum.
fn session_event_from_pg_row(
    row: &sqlx::postgres::PgRow,
    session_id: &SessionId,
) -> Result<SessionEventRecord, HostedStoreError> {
    let event = serde_json::from_value::<SessionEvent>(row.get::<Json<Value>, _>("payload").0)
        .map_err(|_| HostedStoreError::InvalidSession)?;
    Ok(SessionEventRecord {
        id: row.get("id"),
        session_id: session_id.clone(),
        event,
        created_at: row.get("created_at"),
    })
}

/// Returns Unix milliseconds without introducing a database-specific clock
/// type into the shared session domain.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis() as i64
}

fn summary_from_row(row: &sqlx::postgres::PgRow) -> HostedConversationSummary {
    HostedConversationSummary {
        id: row.get("id"),
        title: row.get("title"),
        model: row.get("model"),
        revision: row.get("revision"),
        message_count: row.get("message_count"),
    }
}

fn event_from_row(row: &sqlx::postgres::PgRow) -> HostedChangeEvent {
    HostedChangeEvent {
        id: row.get("id"),
        event_type: row.get("event_type"),
        conversation_id: row.get("conversation_id"),
        conversation_revision: row.get("conversation_revision"),
        payload: row.get::<Json<Value>, _>("payload").0,
    }
}

async fn summary_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    account: &HostedAccount,
    conversation_id: &str,
) -> Result<HostedConversationSummary, HostedStoreError> {
    let row = sqlx::query(
        "SELECT c.id, c.title, c.model, c.revision, COUNT(m.id) AS message_count \
         FROM conversations c LEFT JOIN messages m ON m.conversation_id = c.id \
         WHERE c.account_id = $1 AND c.id = $2 GROUP BY c.id",
    )
    .bind(&account.id)
    .bind(conversation_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(HostedStoreError::NotFound)?;
    Ok(summary_from_row(&row))
}

async fn messages_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    conversation_id: &str,
) -> Result<Vec<HostedMessage>, HostedStoreError> {
    let rows = sqlx::query(
        "SELECT id, parent_message_id, role, content FROM messages \
         WHERE conversation_id = $1 ORDER BY position ASC",
    )
    .bind(conversation_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let part_rows = sqlx::query(
            "SELECT part_type, text_content, image_mime_type, image_bytes \
             FROM message_parts WHERE message_id = $1 ORDER BY position ASC",
        )
        .bind(&id)
        .fetch_all(&mut **transaction)
        .await?;
        let parts = part_rows
            .iter()
            .map(|part| match part.get::<String, _>("part_type").as_str() {
                "text" => HostedMessagePart::Text {
                    text: part.get("text_content"),
                },
                "image" => HostedMessagePart::Image {
                    mime_type: part.get("image_mime_type"),
                    byte_count: part.get::<Vec<u8>, _>("image_bytes").len(),
                },
                _ => HostedMessagePart::Text {
                    text: "[invalid stored message part]".to_string(),
                },
            })
            .collect();
        messages.push(HostedMessage {
            id,
            parent_message_id: row.get("parent_message_id"),
            role: row.get("role"),
            content: row.get("content"),
            parts,
        });
    }
    Ok(messages)
}

/// Reconstructs model-visible parts privately from PostgreSQL. The public
/// hosted conversation read model intentionally exposes image byte counts only;
/// this internal path keeps image bytes on the server while still giving
/// Bifrost the same `MessagePart` foundation used by the local runtime.
async fn model_path_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    conversation_id: &str,
    head_message_id: Option<&str>,
) -> Result<Vec<Message>, HostedStoreError> {
    let rows = sqlx::query(
        "SELECT id, parent_message_id, role, content FROM messages WHERE conversation_id = $1 ORDER BY position ASC",
    )
    .bind(conversation_id)
    .fetch_all(&mut **transaction)
    .await?;
    let nodes = rows
        .iter()
        .map(|row| ConversationTreeNode {
            id: row.get("id"),
            parent_message_id: row.get("parent_message_id"),
        })
        .collect::<Vec<_>>();
    let tree = ConversationTree::new(nodes)?;
    let path = match head_message_id {
        Some(head) => tree.selected_path(head)?,
        None => Vec::new(),
    };
    let by_id = rows
        .iter()
        .map(|row| (row.get::<String, _>("id"), row))
        .collect::<HashMap<_, _>>();
    let mut messages = Vec::with_capacity(path.len());
    for id in path {
        let row = by_id.get(&id).ok_or(HostedStoreError::InvalidTree)?;
        let role = match row.get::<String, _>("role").as_str() {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            _ => return Err(HostedStoreError::InvalidRole),
        };
        let part_rows = sqlx::query(
            "SELECT id, part_type, text_content, image_mime_type, image_bytes FROM message_parts WHERE message_id = $1 ORDER BY position ASC",
        )
        .bind(&id)
        .fetch_all(&mut **transaction)
        .await?;
        let mut parts = Vec::with_capacity(part_rows.len());
        for part in part_rows {
            match part.get::<String, _>("part_type").as_str() {
                "text" => parts.push(MessagePart::Text(part.get("text_content"))),
                "image" => parts.push(MessagePart::Image(ImagePart {
                    asset_id: ImageAssetId::new(part.get::<String, _>("id")),
                    mime_type: part.get("image_mime_type"),
                    bytes: part.get("image_bytes"),
                })),
                _ => return Err(HostedStoreError::InvalidTree),
            }
        }
        messages.push(Message {
            id: Some(crate::conversation::MessageId::new(id)),
            parent_message_id: row
                .get::<Option<String>, _>("parent_message_id")
                .map(crate::conversation::MessageId::new),
            role,
            content: row.get("content"),
            parts,
            metadata: None,
        });
    }
    Ok(messages)
}

/// Loads the minimum graph projection needed to run shared tree policy while
/// the caller still owns the PostgreSQL transaction and conversation lock.
async fn conversation_tree_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    conversation_id: &str,
) -> Result<ConversationTree, HostedStoreError> {
    let rows = sqlx::query(
        "SELECT id, parent_message_id FROM messages WHERE conversation_id = $1 ORDER BY position ASC",
    )
    .bind(conversation_id)
    .fetch_all(&mut **transaction)
    .await?;
    ConversationTree::new(rows.into_iter().map(|row| ConversationTreeNode {
        id: row.get("id"),
        parent_message_id: row.get("parent_message_id"),
    }))
    .map_err(Into::into)
}

/// Converts a read projection into the same validated tree used by both
/// selected-head reads and PostgreSQL mutation planning.
fn conversation_tree_from_messages(
    messages: &[HostedMessage],
) -> Result<ConversationTree, HostedStoreError> {
    ConversationTree::new(messages.iter().map(|message| ConversationTreeNode {
        id: message.id.clone(),
        parent_message_id: message.parent_message_id.clone(),
    }))
    .map_err(Into::into)
}

async fn lock_conversation(
    transaction: &mut Transaction<'_, Postgres>,
    account: &HostedAccount,
    conversation_id: &str,
    expected_revision: i64,
) -> Result<(), HostedStoreError> {
    let actual = sqlx::query_scalar::<_, i64>(
        "SELECT revision FROM conversations WHERE account_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(&account.id)
    .bind(conversation_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(HostedStoreError::NotFound)?;
    if actual != expected_revision {
        return Err(HostedStoreError::StaleRevision {
            expected: expected_revision,
            current: actual,
        });
    }
    Ok(())
}

async fn bump_revision(
    transaction: &mut Transaction<'_, Postgres>,
    conversation_id: &str,
) -> Result<i64, HostedStoreError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "UPDATE conversations SET revision = revision + 1, updated_at = now() \
         WHERE id = $1 RETURNING revision",
    )
    .bind(conversation_id)
    .fetch_one(&mut **transaction)
    .await?)
}

async fn reserve_idempotency(
    transaction: &mut Transaction<'_, Postgres>,
    account: &HostedAccount,
    key: &str,
) -> Result<Option<MutationResponse>, HostedStoreError> {
    if key.trim().is_empty() || key.len() > 255 {
        return Err(HostedStoreError::InvalidIdempotencyKey);
    }
    let inserted = sqlx::query_scalar::<_, bool>(
        "INSERT INTO idempotency_records (account_id, idempotency_key, response_status, response_body) \
         VALUES ($1, $2, 200, '{}'::jsonb) ON CONFLICT DO NOTHING RETURNING TRUE",
    )
    .bind(&account.id)
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await?;
    if inserted.is_some() {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT response_status, response_body FROM idempotency_records \
         WHERE account_id = $1 AND idempotency_key = $2",
    )
    .bind(&account.id)
    .bind(key)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(Some(MutationResponse {
        status: row.get::<i16, _>("response_status") as u16,
        body: row.get::<Json<Value>, _>("response_body").0,
    }))
}

async fn save_idempotency(
    transaction: &mut Transaction<'_, Postgres>,
    account: &HostedAccount,
    key: &str,
    response: &MutationResponse,
) -> Result<(), HostedStoreError> {
    sqlx::query(
        "UPDATE idempotency_records SET response_status = $1, response_body = $2 \
         WHERE account_id = $3 AND idempotency_key = $4",
    )
    .bind(response.status as i16)
    .bind(Json(&response.body))
    .bind(&account.id)
    .bind(key)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn append_event(
    transaction: &mut Transaction<'_, Postgres>,
    account: &HostedAccount,
    event_type: &str,
    conversation_id: Option<&str>,
    conversation_revision: Option<i64>,
    payload: Value,
) -> Result<HostedChangeEvent, HostedStoreError> {
    let row = sqlx::query(
        "INSERT INTO account_change_events \
         (account_id, event_type, conversation_id, conversation_revision, payload) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, event_type, conversation_id, conversation_revision, payload",
    )
    .bind(&account.id)
    .bind(event_type)
    .bind(conversation_id)
    .bind(conversation_revision)
    .bind(Json(payload))
    .fetch_one(&mut **transaction)
    .await?;
    Ok(event_from_row(&row))
}

#[derive(Debug, Clone)]
enum PreparedPart {
    Text(String),
    Image { mime_type: String, bytes: Vec<u8> },
}

fn prepare_parts(parts: &[HostedMessagePartInput]) -> Result<Vec<PreparedPart>, HostedStoreError> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    if parts.is_empty() {
        return Err(HostedStoreError::EmptyMessage);
    }
    let prepared = parts
        .iter()
        .map(|part| match part {
            HostedMessagePartInput::Text { text } => Ok(PreparedPart::Text(text.clone())),
            HostedMessagePartInput::ImageData { mime_type, data } => STANDARD
                .decode(data)
                .map(|bytes| PreparedPart::Image {
                    mime_type: mime_type.clone(),
                    bytes,
                })
                .map_err(|_| HostedStoreError::EmptyMessage),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if prepared
        .iter()
        .all(|part| matches!(part, PreparedPart::Text(text) if text.is_empty()))
    {
        return Err(HostedStoreError::EmptyMessage);
    }
    Ok(prepared)
}

fn prepared_content(parts: &[PreparedPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            PreparedPart::Text(text) => Some(text.as_str()),
            PreparedPart::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn insert_parts(
    transaction: &mut Transaction<'_, Postgres>,
    message_id: &str,
    parts: &[PreparedPart],
) -> Result<(), HostedStoreError> {
    for (position, part) in parts.iter().enumerate() {
        match part {
            PreparedPart::Text(text) => {
                sqlx::query(
                    "INSERT INTO message_parts (id, message_id, position, part_type, text_content) \
                     VALUES ($1, $2, $3, 'text', $4)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(message_id)
                .bind(position as i32)
                .bind(text)
                .execute(&mut **transaction)
                .await?;
            }
            PreparedPart::Image { mime_type, bytes } => {
                sqlx::query(
                    "INSERT INTO message_parts (id, message_id, position, part_type, image_mime_type, image_bytes) \
                     VALUES ($1, $2, $3, 'image', $4, $5)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(message_id)
                .bind(position as i32)
                .bind(mime_type)
                .bind(bytes)
                .execute(&mut **transaction)
                .await?;
            }
        }
    }
    Ok(())
}

async fn copy_message_parts(
    transaction: &mut Transaction<'_, Postgres>,
    old_message_id: &str,
    new_message_id: &str,
) -> Result<(), HostedStoreError> {
    let rows = sqlx::query(
        "SELECT position, part_type, text_content, image_mime_type, image_bytes \
         FROM message_parts WHERE message_id = $1 ORDER BY position ASC",
    )
    .bind(old_message_id)
    .fetch_all(&mut **transaction)
    .await?;
    for row in rows {
        sqlx::query(
            "INSERT INTO message_parts (id, message_id, position, part_type, text_content, image_mime_type, image_bytes) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(new_message_id)
        .bind(row.get::<i32, _>("position"))
        .bind(row.get::<String, _>("part_type"))
        .bind(row.get::<Option<String>, _>("text_content"))
        .bind(row.get::<Option<String>, _>("image_mime_type"))
        .bind(row.get::<Option<Vec<u8>>, _>("image_bytes"))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn selected_path(messages: &[HostedMessage], head: &str) -> Result<Vec<String>, HostedStoreError> {
    conversation_tree_from_messages(messages)?
        .selected_path(head)
        .map_err(|error| match error {
            ConversationTreeError::MessageNotFound(_) => HostedStoreError::InvalidHead,
            other => other.into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_path_follows_parents_from_root() {
        let messages = vec![
            HostedMessage {
                id: "root".into(),
                parent_message_id: None,
                role: "user".into(),
                content: "one".into(),
                parts: vec![],
            },
            HostedMessage {
                id: "child".into(),
                parent_message_id: Some("root".into()),
                role: "assistant".into(),
                content: "two".into(),
                parts: vec![],
            },
        ];
        assert_eq!(
            selected_path(&messages, "child").unwrap(),
            ["root", "child"]
        );
    }

    #[test]
    fn selected_path_rejects_cycles() {
        let messages = vec![HostedMessage {
            id: "one".into(),
            parent_message_id: Some("one".into()),
            role: "user".into(),
            content: "one".into(),
            parts: vec![],
        }];
        assert!(matches!(
            selected_path(&messages, "one"),
            Err(HostedStoreError::InvalidTree)
        ));
    }

    /// Runs every Phase 6 server-side acceptance requirement against an
    /// isolated PostgreSQL database. CI must opt in explicitly so ordinary
    /// unit-test runs cannot touch a hosted project.
    #[tokio::test]
    #[ignore = "requires an isolated WINDIE_HOSTED_TEST_DATABASE_URL"]
    async fn postgres_phase_six_account_sync_acceptance() {
        use crate::conversation::Role;
        use crate::store::Store;

        let database_url = std::env::var("WINDIE_HOSTED_TEST_DATABASE_URL")
            .expect("ignored hosted acceptance test requires an isolated PostgreSQL URL");
        let store = HostedStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let suffix = Uuid::new_v4();
        let subject_a = format!("hosted-test-a-{suffix}");
        let account_a = store.resolve_account(&subject_a).await.unwrap();
        let repeated_account_a = store.resolve_account(&subject_a).await.unwrap();
        let account_b = store
            .resolve_account(&format!("hosted-test-b-{suffix}"))
            .await
            .unwrap();

        // First authenticated requests must map one Supabase subject to one
        // Windie account, even when separate browser requests repeat.
        assert_eq!(account_a.id, repeated_account_a.id);
        let account_rows =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts WHERE auth_subject = $1")
                .bind(&subject_a)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(account_rows, 1);

        let created = store
            .create_conversation(&account_a, "windie/test", "create")
            .await
            .unwrap();
        let conversation_id = created.body["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let creation_cursor = created.body["event_cursor"].as_i64().unwrap();

        // User B cannot list, inspect, mutate, or replay User A's state.
        assert!(
            store
                .list_conversations(&account_b)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        assert!(matches!(
            store.conversation(&account_b, &conversation_id, None).await,
            Err(HostedStoreError::NotFound)
        ));
        assert!(matches!(
            store
                .append_message(
                    &account_b,
                    HostedAppendMutation {
                        conversation_id: &conversation_id,
                        expected_revision: 0,
                        parent_message_id: None,
                        role: "user",
                        parts: &[HostedMessagePartInput::Text {
                            text: "unauthorized".to_string(),
                        }],
                        idempotency_key: "account-b-mutation",
                    },
                )
                .await,
            Err(HostedStoreError::NotFound)
        ));
        assert!(store.events_after(&account_b, 0).await.unwrap().is_empty());

        // A timed-out retry has its original revision but must still produce
        // exactly one message and the originally saved response.
        let root = store
            .append_message(
                &account_a,
                HostedAppendMutation {
                    conversation_id: &conversation_id,
                    expected_revision: 0,
                    parent_message_id: None,
                    role: "user",
                    parts: &[HostedMessagePartInput::Text {
                        text: "hello".to_string(),
                    }],
                    idempotency_key: "append",
                },
            )
            .await
            .unwrap();
        let root_id = root.body["message_id"].as_str().unwrap().to_string();
        let retry = store
            .append_message(
                &account_a,
                HostedAppendMutation {
                    conversation_id: &conversation_id,
                    expected_revision: 0,
                    parent_message_id: None,
                    role: "user",
                    parts: &[HostedMessagePartInput::Text {
                        text: "hello".to_string(),
                    }],
                    idempotency_key: "append",
                },
            )
            .await
            .unwrap();
        assert_eq!(root.body, retry.body);

        // Two independently loaded browser states converge after mutation.
        let browser_one_cursor = store.list_conversations(&account_a).await.unwrap().1;
        let browser_two_cursor = store.list_conversations(&account_a).await.unwrap().1;
        let browser_one = store
            .conversation(&account_a, &conversation_id, None)
            .await
            .unwrap();
        let browser_two = store
            .conversation(&account_a, &conversation_id, None)
            .await
            .unwrap();
        assert_eq!(browser_one.summary.revision, browser_two.summary.revision);
        assert_eq!(browser_one.messages.len(), 1);
        assert_eq!(
            browser_one.messages[0].content,
            browser_two.messages[0].content
        );

        // A reconnect sees saved rows strictly after its cursor and never
        // re-applies an event it has already accepted.
        let root_event = store
            .events_after(&account_a, creation_cursor)
            .await
            .unwrap();
        assert_eq!(root_event.len(), 1);
        assert_eq!(browser_one_cursor, root_event[0].id);
        assert_eq!(browser_two_cursor, root_event[0].id);
        assert!(
            store
                .events_after(&account_a, root_event[0].id)
                .await
                .unwrap()
                .is_empty()
        );

        // Preserve sibling branches from the same parent.
        let branch_a = store
            .append_message(
                &account_a,
                HostedAppendMutation {
                    conversation_id: &conversation_id,
                    expected_revision: 1,
                    parent_message_id: Some(&root_id),
                    role: "assistant",
                    parts: &[HostedMessagePartInput::Text {
                        text: "branch a".to_string(),
                    }],
                    idempotency_key: "branch-a",
                },
            )
            .await
            .unwrap();
        let branch_a_id = branch_a.body["message_id"].as_str().unwrap().to_string();
        let branch_b = store
            .append_message(
                &account_a,
                HostedAppendMutation {
                    conversation_id: &conversation_id,
                    expected_revision: 2,
                    parent_message_id: Some(&root_id),
                    role: "assistant",
                    parts: &[HostedMessagePartInput::Text {
                        text: "branch b".to_string(),
                    }],
                    idempotency_key: "branch-b",
                },
            )
            .await
            .unwrap();
        let branch_b_id = branch_b.body["message_id"].as_str().unwrap().to_string();
        let branch_path = store
            .conversation(&account_a, &conversation_id, Some(&branch_a_id))
            .await
            .unwrap();
        assert_eq!(
            branch_path.selected_path.clone().unwrap(),
            vec![root_id.clone(), branch_a_id.clone()]
        );

        // A destructive operation against an old graph revision must fail.
        assert!(matches!(
            store
                .remove_message(&account_a, &conversation_id, &root_id, 1, "stale-delete")
                .await,
            Err(HostedStoreError::StaleRevision { .. })
        ));

        // Compare hosted branches, forks, and truncation with the existing
        // local SQLite canonical-tree behavior.
        let mut local = Store::open_memory().unwrap();
        let local_conversation = local.create_conversation("windie/test").unwrap();
        let local_root = local
            .insert_message(&local_conversation, None, Role::User, "hello", None)
            .unwrap();
        let local_branch_a = local
            .insert_message(
                &local_conversation,
                Some(&local_root),
                Role::Assistant,
                "branch a",
                None,
            )
            .unwrap();
        let local_branch_b = local
            .insert_message(
                &local_conversation,
                Some(&local_root),
                Role::Assistant,
                "branch b",
                None,
            )
            .unwrap();
        let local_path = local
            .load_path_to_message(&local_conversation, &local_branch_a)
            .unwrap();
        let hosted_path_content = branch_path
            .messages
            .iter()
            .filter(|message| {
                branch_path
                    .selected_path
                    .as_ref()
                    .unwrap()
                    .contains(&message.id)
            })
            .map(|message| (message.role.clone(), message.content.clone()))
            .collect::<Vec<_>>();
        assert_eq!(message_content(&local_path), hosted_path_content);

        let fork = store
            .fork_conversation(&account_a, &conversation_id, &branch_a_id, 3, "fork")
            .await
            .unwrap();
        let fork_id = fork.body["conversation"]["id"].as_str().unwrap();
        let hosted_fork = store.conversation(&account_a, fork_id, None).await.unwrap();
        let local_fork = local
            .fork_conversation_at_message(&local_conversation, &local_branch_a)
            .unwrap();
        assert_eq!(
            message_content(&local.load_message_tree(&local_fork).unwrap()),
            message_content(&hosted_fork.messages)
        );

        let truncated = store
            .truncate_after_message(&account_a, &conversation_id, &root_id, 3, "truncate")
            .await
            .unwrap();
        assert_eq!(truncated.body["revision"], 4);
        local
            .truncate_after_message(&local_conversation, &local_root)
            .unwrap();
        let hosted_after_truncate = store
            .conversation(&account_a, &conversation_id, None)
            .await
            .unwrap();
        assert_eq!(
            message_content(&local.load_message_tree(&local_conversation).unwrap()),
            message_content(&hosted_after_truncate.messages)
        );
        assert_eq!(hosted_after_truncate.messages.len(), 1);
        assert_ne!(branch_a_id, branch_b_id);
        assert_ne!(local_branch_b.as_str(), local_branch_a.as_str());

        // Reopening the pool simulates a server restart. The committed graph
        // and durable event cursor must remain available afterwards.
        let final_cursor = truncated.body["event_cursor"].as_i64().unwrap();
        drop(store);
        let restarted = HostedStore::connect(&database_url).await.unwrap();
        let recovered = restarted
            .conversation(&account_a, &conversation_id, None)
            .await
            .unwrap();
        assert_eq!(recovered.summary.revision, 4);
        assert_eq!(recovered.messages.len(), 1);
        assert!(
            restarted
                .events_after(&account_a, final_cursor)
                .await
                .unwrap()
                .is_empty()
        );

        sqlx::query("DELETE FROM accounts WHERE id = $1 OR id = $2")
            .bind(&account_a.id)
            .bind(&account_b.id)
            .execute(&restarted.pool)
            .await
            .unwrap();
    }

    /// Exercises the Phase 7 persistence contract without contacting Bifrost.
    /// CI must provide a disposable database; this test never targets the
    /// deployed hosted service database.
    #[tokio::test]
    #[ignore = "requires an isolated WINDIE_HOSTED_TEST_DATABASE_URL"]
    async fn postgres_phase_seven_session_execution_acceptance() {
        use crate::session::SessionExecutionStart;

        let database_url = std::env::var("WINDIE_HOSTED_TEST_DATABASE_URL")
            .expect("ignored hosted acceptance test requires an isolated PostgreSQL URL");
        let store = HostedStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let suffix = Uuid::new_v4();
        let account = store
            .resolve_account(&format!("hosted-session-a-{suffix}"))
            .await
            .unwrap();
        let other_account = store
            .resolve_account(&format!("hosted-session-b-{suffix}"))
            .await
            .unwrap();
        let created = store
            .create_conversation(&account, "windie/test", "session-create")
            .await
            .unwrap();
        let conversation_id = created.body["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let root = store
            .append_message(
                &account,
                HostedAppendMutation {
                    conversation_id: &conversation_id,
                    expected_revision: 0,
                    parent_message_id: None,
                    role: "user",
                    parts: &[HostedMessagePartInput::Text {
                        text: "start".to_string(),
                    }],
                    idempotency_key: "session-root",
                },
            )
            .await
            .unwrap();
        let root_id = root.body["message_id"].as_str().unwrap().to_string();

        // Session-head resolution is server-owned and reuses one unique
        // branch rather than creating a duplicate on a second browser request.
        let session = store
            .resolve_or_create_session(&account, &conversation_id, Some(&root_id), None)
            .await
            .unwrap();
        let repeated = store
            .resolve_or_create_session(&account, &conversation_id, Some(&root_id), None)
            .await
            .unwrap();
        assert_eq!(session.id.as_str(), repeated.id.as_str());
        assert!(matches!(
            store.session(&other_account, &session.id).await,
            Err(HostedStoreError::SessionNotFound)
        ));

        // One atomic claim fences a competing worker, and only that token may
        // append the user input or stream records.
        let claim = store
            .claim_session_execution(&account, &session.id, SessionExecutionStart::Runnable)
            .await
            .unwrap();
        // A separate PostgreSQL connection observes only committed event IDs;
        // it reloads the durable record rather than receiving model content in
        // the notification payload.
        let mut listener = store.session_event_listener().await.unwrap();
        assert!(matches!(
            store
                .claim_session_execution(&account, &session.id, SessionExecutionStart::Runnable)
                .await,
            Err(HostedStoreError::SessionConflict)
        ));
        store
            .append_session_input(
                &account,
                &session.id,
                &claim.claim,
                &[
                    HostedMessagePartInput::Text {
                        text: "first input".to_string(),
                    },
                    HostedMessagePartInput::ImageData {
                        mime_type: "image/png".to_string(),
                        data: "AQID".to_string(),
                    },
                ],
            )
            .await
            .unwrap();
        let (_, model_path) = store
            .model_messages_for_claim(&account, &session.id, &claim.claim)
            .await
            .unwrap();
        assert!(matches!(
            model_path.last().unwrap().parts.as_slice(),
            [
                crate::conversation::MessagePart::Text(_),
                crate::conversation::MessagePart::Image(_)
            ]
        ));
        let (queued_id, depth, queued_event) = store
            .enqueue_session_input(
                &account,
                &session.id,
                &[HostedMessagePartInput::Text {
                    text: "second input".to_string(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(depth, 1);
        assert!(matches!(
            queued_event.event,
            SessionEvent::InputQueued { .. }
        ));
        let notification = tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
            .await
            .expect("committed session event should notify other hosted-server instances")
            .unwrap();
        assert_eq!(notification.payload(), queued_event.id.to_string());
        let reloaded = store
            .session_event_by_id(queued_event.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.id, queued_event.id);
        assert!(matches!(reloaded.event, SessionEvent::InputQueued { .. }));
        assert_eq!(
            store
                .session_input_count(&account, &session.id)
                .await
                .unwrap(),
            1
        );

        // A completed turn releases its claim, then the FIFO input becomes the
        // next canonical user node under a new claim.
        let completed = store
            .complete_session_with_assistant(&account, &session.id, &claim.claim, "first reply")
            .await
            .unwrap();
        assert!(matches!(
            completed.last().unwrap().event,
            SessionEvent::Completed { .. }
        ));
        assert!(matches!(
            store
                .append_session_execution_event(
                    &account,
                    &session.id,
                    &claim.claim,
                    SessionEvent::AssistantDelta {
                        text: "stale".to_string()
                    },
                )
                .await,
            Err(HostedStoreError::SessionConflict)
        ));
        let next_claim = store
            .claim_session_execution(&account, &session.id, SessionExecutionStart::Runnable)
            .await
            .unwrap();
        let materialized = store
            .materialize_next_session_input(&account, &session.id, &next_claim.claim)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            materialized.event,
            SessionEvent::InputStarted { ref input_id, .. } if input_id == queued_id.as_str()
        ));
        assert_eq!(
            store
                .session_input_count(&account, &session.id)
                .await
                .unwrap(),
            0
        );

        // Session event cursors replay only later rows, exactly like account
        // event cursors, and a restart treats in-flight work as failure.
        let events = store
            .session_events_after(&account, &session.id, 0)
            .await
            .unwrap();
        let cursor = events.last().unwrap().id;
        assert!(
            store
                .session_events_after(&account, &session.id, cursor)
                .await
                .unwrap()
                .is_empty()
        );
        let recovered = store.recover_interrupted_hosted_sessions().await.unwrap();
        assert!(recovered >= 1);
        assert_eq!(
            store.session(&account, &session.id).await.unwrap().status,
            SessionStatus::Failed
        );

        sqlx::query("DELETE FROM accounts WHERE id = $1 OR id = $2")
            .bind(&account.id)
            .bind(&other_account.id)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    fn message_content<T: MessageContent>(messages: &[T]) -> Vec<(String, String)> {
        messages
            .iter()
            .map(|message| (message.role_text(), message.content_text()))
            .collect()
    }

    trait MessageContent {
        fn role_text(&self) -> String;
        fn content_text(&self) -> String;
    }

    impl MessageContent for HostedMessage {
        fn role_text(&self) -> String {
            self.role.clone()
        }

        fn content_text(&self) -> String {
            self.content.clone()
        }
    }

    impl MessageContent for crate::conversation::Message {
        fn role_text(&self) -> String {
            self.role.as_str().to_string()
        }

        fn content_text(&self) -> String {
            self.content.clone()
        }
    }
}
