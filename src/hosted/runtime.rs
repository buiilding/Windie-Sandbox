//! Hosted session worker and model-execution orchestration.
//!
//! This is intentionally not a second `operation/hosted_session.rs`. It uses
//! shared session domain types and sends their canonical text context to a
//! private Bifrost instance. It owns no local MCP, filesystem, browser, or
//! computer tool execution.

use tokio::{
    sync::mpsc,
    time::{Duration, sleep},
};

use super::{HostedAccount, HostedMessagePartInput, HostedStore, HostedStoreError};
use crate::{
    llm::{BaseUrl, BifrostClient, LlmStreamEvent},
    runtime::progression::active_tool_execution,
    session::{
        Session, SessionEvent, SessionEventHub, SessionExecutionClaim, SessionExecutionStart,
        SessionId, SessionQueryResult, SessionStatus,
    },
};

/// Server-owned executor for hosted model turns.
#[derive(Clone)]
pub(crate) struct HostedRuntime {
    store: HostedStore,
    bifrost_base_url: BaseUrl,
    live_events: SessionEventHub,
}

impl HostedRuntime {
    /// Creates the worker using a private Bifrost endpoint.
    pub(crate) fn new(
        store: HostedStore,
        bifrost_base_url: String,
        live_events: SessionEventHub,
    ) -> Self {
        Self {
            store,
            bifrost_base_url: BaseUrl::new(bifrost_base_url),
            live_events,
        }
    }

    /// Resolves a branch session then accepts input. A running session receives
    /// durable FIFO input; otherwise input is appended under a fresh claim.
    pub(crate) async fn query_conversation(
        &self,
        account: HostedAccount,
        conversation_id: &str,
        head_message_id: Option<&str>,
        parts: Vec<HostedMessagePartInput>,
        reasoning: Option<crate::llm::ReasoningRequest>,
    ) -> Result<SessionQueryResult, HostedStoreError> {
        let session = self
            .store
            .resolve_or_create_session(&account, conversation_id, head_message_id, reasoning)
            .await?;
        self.query_session(account, session, parts).await
    }

    /// Accepts input into one already-resolved account-owned session.
    pub(crate) async fn query_session(
        &self,
        account: HostedAccount,
        session: Session,
        parts: Vec<HostedMessagePartInput>,
    ) -> Result<SessionQueryResult, HostedStoreError> {
        if session.status == SessionStatus::Running {
            let (input_id, queue_depth, record) = self
                .store
                .enqueue_session_input(&account, &session.id, &parts)
                .await?;
            self.live_events.publish(record);
            let session = self.store.session(&account, &session.id).await?;
            return Ok(SessionQueryResult {
                session,
                queued: true,
                input_id: Some(input_id),
                queue_depth,
            });
        }
        if matches!(
            session.status,
            SessionStatus::WaitingForApproval | SessionStatus::WaitingForTool
        ) {
            return Err(HostedStoreError::SessionConflict);
        }
        let claim = self
            .store
            .claim_session_execution(
                &account,
                &session.id,
                SessionExecutionStart::RunnableAtHead(session.current_head_message_id.clone()),
            )
            .await?;
        self.store
            .append_session_input(&account, &session.id, &claim.claim, &parts)
            .await?;
        let session = self.store.session(&account, &session.id).await?;
        self.spawn(account, session.id.clone(), claim.claim);
        Ok(SessionQueryResult {
            session,
            queued: false,
            input_id: None,
            queue_depth: 0,
        })
    }

    /// Starts the resolved session without adding new user input.
    pub(crate) async fn continue_session(
        &self,
        account: HostedAccount,
        session_id: &SessionId,
    ) -> Result<Session, HostedStoreError> {
        let session = self.store.session(&account, session_id).await?;
        if matches!(
            session.status,
            SessionStatus::WaitingForApproval | SessionStatus::WaitingForTool
        ) {
            return Err(HostedStoreError::SessionConflict);
        }
        let claim = self
            .store
            .claim_session_execution(&account, session_id, SessionExecutionStart::Runnable)
            .await?;
        if let Some(record) = self
            .store
            .materialize_next_session_input(&account, session_id, &claim.claim)
            .await?
        {
            self.live_events.publish(record);
        }
        let session = self.store.session(&account, session_id).await?;
        self.spawn(account, session.id.clone(), claim.claim);
        Ok(session)
    }

    /// Fences a running worker and records cancellation durably.
    pub(crate) async fn stop(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> Result<Session, HostedStoreError> {
        let (session, record) = self.store.stop_session(account, session_id).await?;
        self.live_events.publish(record);
        Ok(session)
    }

    /// Marks interrupted hosted work failed at process startup.
    pub(crate) async fn recover_interrupted_sessions(&self) -> Result<u64, HostedStoreError> {
        self.store.recover_interrupted_hosted_sessions().await
    }

    /// Starts the database-backed wakeup scheduler. `SKIP LOCKED` inside the
    /// store lets multiple hosted-server instances run it safely.
    pub(crate) fn start_wakeup_scheduler(&self) {
        let runtime = self.clone();
        tokio::spawn(async move {
            loop {
                match runtime.store.expire_one_pending_device_work().await {
                    Ok(records) => {
                        for record in records {
                            runtime.live_events.publish(record);
                        }
                    }
                    Err(error) => eprintln!("hosted device-work expiry failed: {error}"),
                }
                match runtime.store.claim_due_wakeup().await {
                    Ok(Some((account, claimed, wakeup_id))) => runtime.spawn_with_wakeup(
                        account,
                        claimed.session.id,
                        claimed.claim,
                        Some(wakeup_id),
                    ),
                    Ok(None) => sleep(Duration::from_millis(250)).await,
                    Err(error) => {
                        eprintln!("hosted wakeup scheduler failed: {error}");
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });
    }

    fn spawn(&self, account: HostedAccount, session_id: SessionId, claim: SessionExecutionClaim) {
        self.spawn_with_wakeup(account, session_id, claim, None);
    }

    fn spawn_with_wakeup(
        &self,
        account: HostedAccount,
        session_id: SessionId,
        claim: SessionExecutionClaim,
        wakeup_id: Option<String>,
    ) {
        let runtime = self.clone();
        tokio::spawn(async move {
            runtime.run(account, session_id, claim, wakeup_id).await;
        });
    }

    async fn run(
        &self,
        account: HostedAccount,
        session_id: SessionId,
        claim: SessionExecutionClaim,
        wakeup_id: Option<String>,
    ) {
        let result = self.run_claimed(&account, &session_id, &claim).await;
        if let Err(error) = result {
            if let Ok(Some(record)) = self
                .store
                .fail_claimed_session(&account, &session_id, &claim, &error.to_string())
                .await
            {
                self.live_events.publish(record);
            }
            if let Some(wakeup_id) = wakeup_id {
                let _ = self.store.finish_wakeup(&wakeup_id, &claim, false).await;
            }
            return;
        }
        if let Some(wakeup_id) = wakeup_id {
            let _ = self.store.finish_wakeup(&wakeup_id, &claim, true).await;
        }
        let _ = self.start_next_queued(account, session_id).await;
    }

    async fn run_claimed(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &SessionExecutionClaim,
    ) -> Result<(), HostedStoreError> {
        let (session, context) = self
            .store
            .model_context_for_claim(account, session_id, claim)
            .await?;
        if let Some(active) = active_tool_execution(&context.messages) {
            if let Some(call) = active.next_pending_tool_call() {
                let records = self
                    .store
                    .park_existing_device_tool_approval(
                        account,
                        session_id,
                        claim,
                        active.assistant_message_id.as_str(),
                        active.result_parent_message_id.as_str(),
                        call,
                        "tool requires approval",
                    )
                    .await?;
                for record in records {
                    self.live_events.publish(record);
                }
                return Ok(());
            }
        }
        let client = BifrostClient::new(
            self.bifrost_base_url.clone(),
            crate::llm::ModelName::new(session.model),
        );
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let event_store = self.store.clone();
        let event_account = account.clone();
        let event_session = session_id.clone();
        let event_claim = claim.clone();
        let event_hub = self.live_events.clone();
        let event_writer = tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let record = event_store
                    .append_session_execution_event(
                        &event_account,
                        &event_session,
                        &event_claim,
                        event,
                    )
                    .await?;
                event_hub.publish(record);
            }
            Ok::<(), HostedStoreError>(())
        });
        let response = client
            .stream(
                &context.messages,
                &context.tool_schemas,
                session.reasoning.as_ref(),
                None,
                |event| {
                    let event = match event {
                        LlmStreamEvent::AssistantDelta(text) => SessionEvent::AssistantDelta {
                            text: text.to_string(),
                        },
                        LlmStreamEvent::ReasoningDelta(text) => SessionEvent::ReasoningDelta {
                            text: text.to_string(),
                        },
                        LlmStreamEvent::ToolCallDelta {
                            index,
                            id,
                            name,
                            arguments_delta,
                        } => SessionEvent::ToolCallDelta {
                            index,
                            id: id.map(ToOwned::to_owned),
                            name: name.map(ToOwned::to_owned),
                            arguments_delta: arguments_delta.map(ToOwned::to_owned),
                        },
                    };
                    sender
                        .send(event)
                        .map_err(|_| anyhow::anyhow!("hosted session event writer stopped"))
                },
            )
            .await;
        drop(sender);
        event_writer.await.map_err(|error| {
            HostedStoreError::Database(sqlx::Error::Protocol(error.to_string()))
        })??;
        let response = response.map_err(|error| {
            HostedStoreError::Database(sqlx::Error::Protocol(error.to_string()))
        })?;
        if let Some(call) = response
            .metadata
            .tool_calls
            .iter()
            .min_by_key(|call| call.index)
        {
            // The store reuses the shared policy decision and persists a
            // manual approval wait. Streaming/model work ends here; only a
            // later browser approval can create a device assignment.
            let records = match self
                .store
                .park_claimed_session_for_device_approval(
                    account,
                    session_id,
                    claim,
                    &response.content,
                    &response.metadata,
                    call,
                    "tool requires approval",
                )
                .await
            {
                Ok(records) => records,
                // A tool can disappear or become stale between model context
                // assembly and this transaction. Persist the shared-policy
                // failure so the model gets an honest result on continuation.
                Err(HostedStoreError::SessionConflict) => {
                    self.store
                        .park_claimed_session_with_denied_tool(
                            account,
                            session_id,
                            claim,
                            &response.content,
                            &response.metadata,
                            call,
                            "attached tool is unavailable on the bound device",
                        )
                        .await?
                }
                Err(error) => return Err(error),
            };
            for record in records {
                self.live_events.publish(record);
            }
            return Ok(());
        }
        let records = self
            .store
            .complete_session_with_assistant_metadata(
                account,
                session_id,
                claim,
                &response.content,
                &response.metadata,
            )
            .await?;
        for record in records {
            self.live_events.publish(record);
        }
        Ok(())
    }

    async fn start_next_queued(&self, account: HostedAccount, session_id: SessionId) -> bool {
        let Ok(depth) = self.store.session_input_count(&account, &session_id).await else {
            return false;
        };
        if depth == 0 {
            return false;
        }
        let Ok(claim) = self
            .store
            .claim_session_execution(&account, &session_id, SessionExecutionStart::Runnable)
            .await
        else {
            return false;
        };
        let Ok(Some(record)) = self
            .store
            .materialize_next_session_input(&account, &session_id, &claim.claim)
            .await
        else {
            return false;
        };
        self.live_events.publish(record);
        self.spawn(account, session_id, claim.claim);
        true
    }
}
