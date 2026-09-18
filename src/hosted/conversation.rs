//! Hosted API-facing conversation adapter.
//!
//! This layer converts hosted HTTP commands into account-scoped PostgreSQL
//! calls. It deliberately owns no canonical tree policy: shared path, splice,
//! and truncation rules live in `conversation::tree` and are applied inside the
//! SQLite and PostgreSQL transaction adapters.

use serde_json::Value;

use super::{
    HostedAccount, HostedChangeEvent, HostedConversation, HostedConversationSummary,
    HostedMessagePartInput, HostedStore, HostedStoreError, MutationResponse,
    store::HostedAppendMutation,
};

/// Hosted conversation workflow service.
///
/// It accepts account identity resolved by hosted authentication. Callers never
/// provide an account ID from request input.
#[derive(Clone)]
pub(crate) struct HostedConversationOperations {
    store: HostedStore,
}

impl HostedConversationOperations {
    /// Creates workflows backed by one private PostgreSQL store.
    pub(crate) fn new(store: HostedStore) -> Self {
        Self { store }
    }

    /// Lists the authenticated account's conversation summaries and event cursor.
    pub(crate) async fn list(
        &self,
        account: &HostedAccount,
    ) -> Result<(Vec<HostedConversationSummary>, i64), HostedStoreError> {
        self.store.list_conversations(account).await
    }

    /// Loads one canonical account-owned message tree and optional selected path.
    pub(crate) async fn load(
        &self,
        account: &HostedAccount,
        conversation_id: &str,
        selected_head: Option<&str>,
    ) -> Result<HostedConversation, HostedStoreError> {
        self.store
            .conversation(account, conversation_id, selected_head)
            .await
    }

    /// Creates an empty account-owned conversation with a replay-safe mutation key.
    pub(crate) async fn create(
        &self,
        account: &HostedAccount,
        command: CreateHostedConversation,
    ) -> Result<MutationResponse, HostedStoreError> {
        let model = command.model.unwrap_or_else(|| "windie/hosted".to_string());
        self.store
            .create_conversation(account, &model, &command.idempotency_key)
            .await
    }

    /// Adds a canonical parent-linked message under an explicit selected head.
    pub(crate) async fn append(
        &self,
        account: &HostedAccount,
        command: AppendHostedMessage,
    ) -> Result<MutationResponse, HostedStoreError> {
        let AppendHostedMessage {
            conversation_id,
            expected_revision,
            parent_message_id,
            role,
            text,
            parts,
            idempotency_key,
        } = command;
        let parts = message_parts(text, parts);
        self.store
            .append_message(
                account,
                HostedAppendMutation {
                    conversation_id: &conversation_id,
                    expected_revision,
                    parent_message_id: parent_message_id.as_deref(),
                    role: &role,
                    parts: &parts,
                    idempotency_key: &idempotency_key,
                },
            )
            .await
    }

    /// Replaces one message while enforcing the client's expected revision.
    pub(crate) async fn update(
        &self,
        account: &HostedAccount,
        command: UpdateHostedMessage,
    ) -> Result<MutationResponse, HostedStoreError> {
        self.store
            .update_message(
                account,
                &command.conversation_id,
                &command.message_id,
                command.expected_revision,
                &command.text,
                &command.idempotency_key,
            )
            .await
    }

    /// Removes one message and preserves the canonical tree's surviving branch links.
    pub(crate) async fn remove(
        &self,
        account: &HostedAccount,
        command: RemoveHostedMessage,
    ) -> Result<MutationResponse, HostedStoreError> {
        self.store
            .remove_message(
                account,
                &command.conversation_id,
                &command.message_id,
                command.expected_revision,
                &command.idempotency_key,
            )
            .await
    }

    /// Removes descendants after a checkpoint without removing the checkpoint.
    pub(crate) async fn truncate(
        &self,
        account: &HostedAccount,
        command: TruncateHostedConversation,
    ) -> Result<MutationResponse, HostedStoreError> {
        self.store
            .truncate_after_message(
                account,
                &command.conversation_id,
                &command.message_id,
                command.expected_revision,
                &command.idempotency_key,
            )
            .await
    }

    /// Forks the selected root-to-head path into a separate account-owned conversation.
    pub(crate) async fn fork(
        &self,
        account: &HostedAccount,
        command: ForkHostedConversation,
    ) -> Result<MutationResponse, HostedStoreError> {
        self.store
            .fork_conversation(
                account,
                &command.conversation_id,
                &command.message_id,
                command.expected_revision,
                &command.idempotency_key,
            )
            .await
    }

    /// Reads durable account events strictly after a browser's accepted cursor.
    pub(crate) async fn events_after(
        &self,
        account: &HostedAccount,
        after: i64,
    ) -> Result<Vec<HostedChangeEvent>, HostedStoreError> {
        self.store.events_after(account, after).await
    }
}

/// Input for a replay-safe hosted conversation creation.
pub(crate) struct CreateHostedConversation {
    pub(crate) model: Option<String>,
    pub(crate) idempotency_key: String,
}

/// Input for a replay-safe parent-linked hosted message insertion.
pub(crate) struct AppendHostedMessage {
    pub(crate) conversation_id: String,
    pub(crate) expected_revision: i64,
    pub(crate) parent_message_id: Option<String>,
    pub(crate) role: String,
    pub(crate) text: Option<String>,
    pub(crate) parts: Vec<HostedMessagePartInput>,
    pub(crate) idempotency_key: String,
}

/// Uses text as one text part only when the caller supplied no richer parts.
pub(crate) fn message_parts(
    text: Option<String>,
    parts: Vec<HostedMessagePartInput>,
) -> Vec<HostedMessagePartInput> {
    if parts.is_empty() {
        text.map(|text| vec![HostedMessagePartInput::Text { text }])
            .unwrap_or_default()
    } else {
        parts
    }
}

/// Input for a replay-safe hosted message replacement.
pub(crate) struct UpdateHostedMessage {
    pub(crate) conversation_id: String,
    pub(crate) message_id: String,
    pub(crate) expected_revision: i64,
    pub(crate) text: String,
    pub(crate) idempotency_key: String,
}

/// Input for a replay-safe hosted message deletion.
pub(crate) struct RemoveHostedMessage {
    pub(crate) conversation_id: String,
    pub(crate) message_id: String,
    pub(crate) expected_revision: i64,
    pub(crate) idempotency_key: String,
}

/// Input for a replay-safe hosted descendant truncation.
pub(crate) struct TruncateHostedConversation {
    pub(crate) conversation_id: String,
    pub(crate) message_id: String,
    pub(crate) expected_revision: i64,
    pub(crate) idempotency_key: String,
}

/// Input for a replay-safe hosted selected-path fork.
pub(crate) struct ForkHostedConversation {
    pub(crate) conversation_id: String,
    pub(crate) message_id: String,
    pub(crate) expected_revision: i64,
    pub(crate) idempotency_key: String,
}

/// Returns the API-ready response body stored by a successful idempotent mutation.
pub(crate) fn mutation_body(response: MutationResponse) -> (u16, Value) {
    (response.status, response.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_uses_text_when_parts_are_omitted() {
        let command = AppendHostedMessage {
            conversation_id: "conversation".to_string(),
            expected_revision: 0,
            parent_message_id: None,
            role: "user".to_string(),
            text: Some("hello".to_string()),
            parts: vec![],
            idempotency_key: "key".to_string(),
        };

        assert!(matches!(
            message_parts(command.text, command.parts).as_slice(),
            [HostedMessagePartInput::Text { text }] if text == "hello"
        ));
    }

    #[test]
    fn append_keeps_explicit_parts_over_legacy_text() {
        let command = AppendHostedMessage {
            conversation_id: "conversation".to_string(),
            expected_revision: 0,
            parent_message_id: None,
            role: "user".to_string(),
            text: Some("ignored".to_string()),
            parts: vec![HostedMessagePartInput::Text {
                text: "kept".to_string(),
            }],
            idempotency_key: "key".to_string(),
        };

        assert!(matches!(
            message_parts(command.text, command.parts).as_slice(),
            [HostedMessagePartInput::Text { text }] if text == "kept"
        ));
    }
}
