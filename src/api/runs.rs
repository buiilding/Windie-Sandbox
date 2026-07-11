//! Backend-owned runtime run lifecycle and reconnectable SSE routes.

use super::*;
use crate::store::RuntimeRunStatus;

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/conversations/{conversation_id}/runs",
            post(start_query_run),
        )
        .route(
            "/api/conversations/{conversation_id}/active-run",
            get(active_conversation_run),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/approve-run",
            post(start_approve_run),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/deny-run",
            post(start_deny_run),
        )
        .route("/api/runs/{run_id}", get(get_run))
        .route("/api/runs/{run_id}/events", get(run_events))
        .route("/api/runs/{run_id}/cancel", post(cancel_run))
}

/// Runtime action that can be driven through the shared event stream.
enum RuntimeStreamAction {
    Query {
        conversation_id: ConversationId,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
    },
    ApproveTool {
        conversation_id: ConversationId,
        tool_call_id: ToolCallId,
    },
    DenyTool {
        conversation_id: ConversationId,
        tool_call_id: ToolCallId,
    },
}

impl RuntimeStreamAction {
    /// Returns the conversation that owns this runtime action.
    fn conversation_id(&self) -> &ConversationId {
        match self {
            Self::Query {
                conversation_id, ..
            }
            | Self::ApproveTool {
                conversation_id, ..
            }
            | Self::DenyTool {
                conversation_id, ..
            } => conversation_id,
        }
    }

    fn run_action(&self) -> RuntimeRunAction {
        match self {
            Self::Query { .. } => RuntimeRunAction::Query,
            Self::ApproveTool { .. } => RuntimeRunAction::ApproveTool,
            Self::DenyTool { .. } => RuntimeRunAction::DenyTool,
        }
    }
}

/// Starts a backend-owned query and returns immediately with its durable ID.
async fn start_query_run(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<QueryRequest>,
) -> ApiResult<RunSnapshot> {
    let snapshot = start_runtime_run(
        state,
        RuntimeStreamAction::Query {
            conversation_id: ConversationId::new(conversation_id),
            model_override: request.model_override(),
            reasoning: request.reasoning(),
        },
    )
    .await?;

    Ok(Json(snapshot))
}

/// Starts a backend-owned approval continuation.
async fn start_approve_run(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<RunSnapshot> {
    let snapshot = start_runtime_run(
        state,
        RuntimeStreamAction::ApproveTool {
            conversation_id: ConversationId::new(conversation_id),
            tool_call_id: ToolCallId::new(tool_call_id),
        },
    )
    .await?;

    Ok(Json(snapshot))
}

/// Starts a backend-owned denial continuation.
async fn start_deny_run(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<RunSnapshot> {
    let snapshot = start_runtime_run(
        state,
        RuntimeStreamAction::DenyTool {
            conversation_id: ConversationId::new(conversation_id),
            tool_call_id: ToolCallId::new(tool_call_id),
        },
    )
    .await?;

    Ok(Json(snapshot))
}

/// Returns current state for a durable run.
async fn get_run(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
) -> ApiResult<RunSnapshot> {
    Ok(Json(state.run_manager.snapshot(&run_id).await?))
}

#[derive(Debug, Serialize)]
/// Nullable active-run response used when an inspector reloads.
struct ActiveRunResponse {
    run: Option<RunSnapshot>,
}

/// Returns the active backend run for one conversation.
async fn active_conversation_run(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<ActiveRunResponse> {
    Ok(Json(ActiveRunResponse {
        run: state
            .run_manager
            .active_for_conversation(&ConversationId::new(conversation_id))
            .await?,
    }))
}

/// Explicitly cancels one backend-owned run.
async fn cancel_run(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
) -> ApiResult<RunSnapshot> {
    Ok(Json(state.run_manager.cancel(&run_id).await?))
}

/// Starts one task whose lifetime is independent from HTTP subscribers.
async fn start_runtime_run(state: ApiState, action: RuntimeStreamAction) -> Result<RunSnapshot> {
    let snapshot = state
        .run_manager
        .begin_action(action.conversation_id(), action.run_action())
        .await?;
    let run_id = snapshot.id.clone();
    let task_run_id = run_id.clone();
    let manager = state.run_manager.clone();
    let task_manager = manager.clone();
    manager.spawn_supervised(task_run_id.clone(), async move {
        let result = async {
            let mut store = open_store(&state)?;
            let pending_writes = PendingRunWrites::default();
            let events = PersistentRunEventSink {
                manager: task_manager.clone(),
                run_id: task_run_id.clone(),
                pending_writes: pending_writes.clone(),
            };
            let output = PersistentRunOutput {
                manager: task_manager.clone(),
                run_id: task_run_id.clone(),
                buffered_delta: std::sync::Mutex::new(None),
                pending_writes: pending_writes.clone(),
            };
            let message = match action {
                RuntimeStreamAction::Query {
                    conversation_id,
                    model_override,
                    reasoning,
                } => {
                    let runtime =
                        runtime_turn_config(&state, &task_run_id, model_override, reasoning)?;
                    operation::query_runtime_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        runtime,
                    )
                    .await
                    .map(Some)
                }
                RuntimeStreamAction::ApproveTool {
                    conversation_id,
                    tool_call_id,
                } => {
                    let runtime = runtime_turn_config(&state, &task_run_id, None, None)?;
                    operation::approve_tool_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        &tool_call_id,
                        runtime,
                    )
                    .await
                }
                RuntimeStreamAction::DenyTool {
                    conversation_id,
                    tool_call_id,
                } => {
                    let runtime = runtime_turn_config(&state, &task_run_id, None, None)?;
                    operation::deny_tool_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        &tool_call_id,
                        runtime,
                    )
                    .await
                }
            };
            output.flush()?;
            pending_writes.flush().await?;

            message
        }
        .await;

        match result {
            Ok(message) => {
                let message_id = message
                    .and_then(|message| message.id)
                    .map(|id| id.as_str().to_string());
                if let Err(error) = task_manager.complete(&task_run_id, message_id).await {
                    log_api_error(&error);
                }
            }
            Err(error) => {
                log_api_error(&error);
                let persist_result = if is_runtime_cancelled(&error) {
                    open_store(&state)
                        .and_then(|store| {
                            store.interrupt_tool_call_executions_for_run(&task_run_id)?;
                            Ok(())
                        })
                        .map(|_| ())
                } else {
                    Ok(())
                };
                let persist_result = match persist_result {
                    Ok(()) if is_runtime_cancelled(&error) => {
                        task_manager.finish_cancelled(&task_run_id).await
                    }
                    Ok(()) => {
                        task_manager
                            .fail(
                                &task_run_id,
                                raw_error_message(&error),
                                error_causes(&error),
                            )
                            .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(persist_error) = persist_result {
                    log_api_error(&persist_error);
                }
            }
        }
    });

    Ok(snapshot)
}

#[derive(Debug, Deserialize)]
/// Cursor used to replay only events a client has not already rendered.
struct RunEventsQuery {
    #[serde(default)]
    after: u64,
}

/// Replays stored events and then follows the active run until terminal state.
async fn run_events(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
    Query(query): Query<RunEventsQuery>,
) -> std::result::Result<
    Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>,
    ApiError,
> {
    let subscription = state.run_manager.subscribe(&run_id, query.after).await?;

    Ok(persistent_run_event_sse(
        subscription,
        state.run_manager,
        run_id,
        query.after,
    ))
}

/// Converts persisted and live run events into reconnectable SSE frames.
fn persistent_run_event_sse(
    subscription: RunSubscription,
    manager: Arc<RunManager>,
    run_id: String,
    after: u64,
) -> Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>> {
    let stream = stream::unfold(
        PersistentRunSseState {
            pending: VecDeque::from(subscription.history),
            receiver: subscription.receiver,
            manager,
            run_id,
            after,
            terminal_sent: false,
        },
        |mut state| async move {
            loop {
                if state.terminal_sent {
                    return None;
                }

                if let Some(envelope) = state.pending.pop_front() {
                    if envelope.sequence <= state.after {
                        continue;
                    }
                    state.after = envelope.sequence;
                    state.terminal_sent = envelope.event.is_terminal();
                    let event = run_event_frame(&envelope);
                    return Some((Ok::<Event, Infallible>(event), state));
                }

                match state.receiver.recv().await {
                    Ok(envelope) => {
                        if envelope.sequence > state.after {
                            state.pending.push_back(envelope);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        match state.manager.events_after(&state.run_id, state.after).await {
                            Ok(events) => state.pending.extend(events),
                            Err(error) => {
                                state.terminal_sent = true;
                                let envelope = RunEventEnvelope {
                                    run_id: state.run_id.clone(),
                                    sequence: state.after.saturating_add(1),
                                    event: RunEvent::QueryError {
                                        error: raw_error_message(&error),
                                        causes: error_causes(&error),
                                    },
                                };
                                return Some((
                                    Ok::<Event, Infallible>(run_event_frame(&envelope)),
                                    state,
                                ));
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        match state.manager.events_after(&state.run_id, state.after).await {
                            Ok(events) if !events.is_empty() => state.pending.extend(events),
                            Ok(_) => match state.manager.snapshot(&state.run_id).await {
                                Ok(snapshot) if snapshot.status == RuntimeRunStatus::Running => {
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                }
                                _ => return None,
                            },
                            Err(_) => return None,
                        }
                    }
                }
            }
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Serializes one run event with both SSE and JSON sequence metadata.
fn run_event_frame(envelope: &RunEventEnvelope) -> Event {
    let data = serde_json::to_string(envelope).unwrap_or_else(|error| {
        serde_json::json!({
            "run_id": envelope.run_id,
            "sequence": envelope.sequence,
            "type": "query_error",
            "error": format!("failed to serialize runtime event: {error}"),
            "causes": [format!("failed to serialize runtime event: {error}")],
        })
        .to_string()
    });

    Event::default()
        .id(envelope.sequence.to_string())
        .event(envelope.event.event_name())
        .data(data)
}

/// Subscriber state survives for one HTTP connection but owns no runtime task.
struct PersistentRunSseState {
    pending: VecDeque<RunEventEnvelope>,
    receiver: broadcast::Receiver<RunEventEnvelope>,
    manager: Arc<RunManager>,
    run_id: String,
    after: u64,
    terminal_sent: bool,
}

/// Runtime output sink used by non-streaming API query execution.
///
/// The plain `query` endpoint returns one final JSON message, so live model
/// Persists durable message notifications for a backend-owned run.
struct PersistentRunEventSink {
    manager: Arc<RunManager>,
    run_id: String,
    pending_writes: PendingRunWrites,
}

impl RuntimeEventSink for PersistentRunEventSink {
    fn assistant_message_saved(&self, message_id: &MessageId) -> Result<()> {
        self.pending_writes.push(self.manager.enqueue(
            &self.run_id,
            RunEvent::AssistantMessageSaved {
                message_id: message_id.as_str().to_string(),
            },
        )?)
    }

    fn tool_result_saved(&self, message_id: &MessageId) -> Result<()> {
        self.pending_writes.push(self.manager.enqueue(
            &self.run_id,
            RunEvent::ToolResultSaved {
                message_id: message_id.as_str().to_string(),
            },
        )?)
    }
}

#[derive(Clone, Default)]
struct PendingRunWrites {
    writes: Arc<std::sync::Mutex<Vec<crate::run::PendingRunEvent>>>,
}

impl PendingRunWrites {
    fn push(&self, write: crate::run::PendingRunEvent) -> Result<()> {
        self.writes
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime write receipt lock was poisoned"))?
            .push(write);
        Ok(())
    }

    async fn flush(&self) -> Result<()> {
        let writes = std::mem::take(
            &mut *self
                .writes
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime write receipt lock was poisoned"))?,
        );
        for write in writes {
            write.persisted().await?;
        }
        Ok(())
    }
}

/// Persists live display deltas so a reloaded UI can replay the active output.
struct PersistentRunOutput {
    manager: Arc<RunManager>,
    run_id: String,
    buffered_delta: std::sync::Mutex<Option<BufferedRunDelta>>,
    pending_writes: PendingRunWrites,
}

const RUN_DELTA_FLUSH_BYTES: usize = 512;

enum BufferedRunDelta {
    Assistant(String),
    Reasoning(String),
    ToolCall {
        index: u16,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
}

impl BufferedRunDelta {
    fn merge(&mut self, next: &Self) -> bool {
        match (self, next) {
            (Self::Assistant(current), Self::Assistant(next))
            | (Self::Reasoning(current), Self::Reasoning(next)) => {
                current.push_str(next);
                true
            }
            (
                Self::ToolCall {
                    index,
                    id,
                    name,
                    arguments,
                },
                Self::ToolCall {
                    index: next_index,
                    id: next_id,
                    name: next_name,
                    arguments: next_arguments,
                },
            ) if index == next_index => {
                if id.is_none() {
                    *id = next_id.clone();
                }
                if name.is_none() {
                    *name = next_name.clone();
                }
                arguments.push_str(next_arguments);
                true
            }
            _ => false,
        }
    }

    fn byte_len(&self) -> usize {
        match self {
            Self::Assistant(text) | Self::Reasoning(text) => text.len(),
            Self::ToolCall {
                id,
                name,
                arguments,
                ..
            } => {
                id.as_ref().map_or(0, String::len)
                    + name.as_ref().map_or(0, String::len)
                    + arguments.len()
            }
        }
    }

    fn into_event(self) -> RunEvent {
        match self {
            Self::Assistant(text) => RunEvent::AssistantDelta { text },
            Self::Reasoning(text) => RunEvent::ReasoningDelta { text },
            Self::ToolCall {
                index,
                id,
                name,
                arguments,
            } => RunEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta: (!arguments.is_empty()).then_some(arguments),
            },
        }
    }
}

impl PersistentRunOutput {
    fn push_delta(&self, delta: BufferedRunDelta) -> Result<()> {
        let flush = {
            let mut buffered = self
                .buffered_delta
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime delta buffer lock was poisoned"))?;
            if let Some(current) = buffered.as_mut()
                && current.merge(&delta)
            {
                if current.byte_len() < RUN_DELTA_FLUSH_BYTES {
                    return Ok(());
                }
                buffered.take()
            } else {
                buffered.replace(delta)
            }
        };
        if let Some(delta) = flush {
            self.pending_writes
                .push(self.manager.enqueue(&self.run_id, delta.into_event())?)?;
        }
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        let delta = self
            .buffered_delta
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime delta buffer lock was poisoned"))?
            .take();
        if let Some(delta) = delta {
            self.pending_writes
                .push(self.manager.enqueue(&self.run_id, delta.into_event())?)?;
        }
        Ok(())
    }
}

impl RuntimeOutput for PersistentRunOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.push_delta(BufferedRunDelta::Assistant(text.to_string()))
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.push_delta(BufferedRunDelta::Reasoning(text.to_string()))
    }

    fn tool_call_delta(
        &self,
        index: u16,
        id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Result<()> {
        self.push_delta(BufferedRunDelta::ToolCall {
            index,
            id: id.map(str::to_string),
            name: name.map(str::to_string),
            arguments: arguments_delta.unwrap_or_default().to_string(),
        })
    }

    fn end_assistant_message(&self) {
        if let Err(error) = self.flush() {
            log_api_error(&error);
        }
    }

    fn assistant_tool_calls(&self, _tool_calls: &[ToolCall]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn durable_output_coalesces_small_deltas_without_losing_text() {
        let path = std::env::temp_dir().join(format!(
            "windie-run-delta-buffer-{}-{}.db",
            std::process::id(),
            Uuid::new_v4()
        ));
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = Arc::new(RunManager::new(Some(path.clone())).unwrap());
        let run = manager.begin(&conversation_id).await.unwrap();
        let pending_writes = PendingRunWrites::default();
        let output = PersistentRunOutput {
            manager: Arc::clone(&manager),
            run_id: run.id.clone(),
            buffered_delta: std::sync::Mutex::new(None),
            pending_writes: pending_writes.clone(),
        };

        for _ in 0..100 {
            output.assistant_delta("x").unwrap();
        }
        output.flush().unwrap();
        pending_writes.flush().await.unwrap();
        manager.complete(&run.id, None).await.unwrap();

        let events = manager.events_after(&run.id, 0).await.unwrap();
        let deltas = events
            .iter()
            .filter_map(|event| match &event.event {
                RunEvent::AssistantDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(deltas, vec!["x".repeat(100)]);
        assert_eq!(events.len(), 2);

        drop(output);
        drop(manager);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn sse_polls_events_owned_by_another_run_manager() {
        let path = std::env::temp_dir().join(format!(
            "windie-run-nonlocal-sse-{}-{}.db",
            std::process::id(),
            Uuid::new_v4()
        ));
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let owner = Arc::new(RunManager::new(Some(path.clone())).unwrap());
        let follower = Arc::new(RunManager::new(Some(path.clone())).unwrap());
        let run = owner.begin(&conversation_id).await.unwrap();
        let subscription = follower.subscribe(&run.id, 0).await.unwrap();
        let response =
            persistent_run_event_sse(subscription, Arc::clone(&follower), run.id.clone(), 0)
                .into_response();
        let owner_for_task = Arc::clone(&owner);
        let run_id = run.id.clone();
        let producer = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            owner_for_task
                .publish(
                    &run_id,
                    RunEvent::AssistantDelta {
                        text: "remote".to_string(),
                    },
                )
                .await
                .unwrap();
            owner_for_task.complete(&run_id, None).await.unwrap();
        });

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        producer.await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("event: assistant_delta"));
        assert!(body.contains("event: query_done"));

        drop(follower);
        drop(owner);
        let _ = std::fs::remove_file(path);
    }
}
