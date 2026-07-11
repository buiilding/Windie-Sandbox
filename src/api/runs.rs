//! Backend-owned runtime run lifecycle and reconnectable SSE routes.

use super::*;

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
    )?;

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
    )?;

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
    )?;

    Ok(Json(snapshot))
}

/// Returns current state for a durable run.
async fn get_run(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
) -> ApiResult<RunSnapshot> {
    Ok(Json(state.run_manager.snapshot(&run_id)?))
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
            .active_for_conversation(&ConversationId::new(conversation_id))?,
    }))
}

/// Explicitly cancels one backend-owned run.
async fn cancel_run(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
) -> ApiResult<RunSnapshot> {
    Ok(Json(state.run_manager.cancel(&run_id)?))
}

/// Starts one task whose lifetime is independent from HTTP subscribers.
fn start_runtime_run(state: ApiState, action: RuntimeStreamAction) -> Result<RunSnapshot> {
    let snapshot = state.run_manager.begin(action.conversation_id())?;
    let run_id = snapshot.id.clone();
    let task_run_id = run_id.clone();
    let manager = state.run_manager.clone();
    let registration_manager = manager.clone();
    let task = tokio::spawn(async move {
        let result = async {
            let mut store = open_store(&state)?;
            let events = PersistentRunEventSink {
                manager: manager.clone(),
                run_id: task_run_id.clone(),
            };
            let output = PersistentRunOutput {
                manager: manager.clone(),
                run_id: task_run_id.clone(),
            };
            let message = match action {
                RuntimeStreamAction::Query {
                    conversation_id,
                    model_override,
                    reasoning,
                } => {
                    let runtime = runtime_turn_config(&state, model_override, reasoning);
                    operation::query_runtime_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        runtime,
                    )
                    .await
                    .map(Some)?
                }
                RuntimeStreamAction::ApproveTool {
                    conversation_id,
                    tool_call_id,
                } => {
                    let runtime = runtime_turn_config(&state, None, None);
                    operation::approve_tool_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        &tool_call_id,
                        runtime,
                    )
                    .await?
                }
                RuntimeStreamAction::DenyTool {
                    conversation_id,
                    tool_call_id,
                } => {
                    let runtime = runtime_turn_config(&state, None, None);
                    operation::deny_tool_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        &tool_call_id,
                        runtime,
                    )
                    .await?
                }
            };

            Ok::<Option<Message>, anyhow::Error>(message)
        }
        .await;

        match result {
            Ok(message) => {
                let message_id = message
                    .and_then(|message| message.id)
                    .map(|id| id.as_str().to_string());
                if let Err(error) = manager.complete(&task_run_id, message_id) {
                    log_api_error(&error);
                }
            }
            Err(error) => {
                log_api_error(&error);
                if let Err(persist_error) = manager.fail(
                    &task_run_id,
                    raw_error_message(&error),
                    error_causes(&error),
                ) {
                    log_api_error(&persist_error);
                }
            }
        }
    });
    registration_manager.register_task(&run_id, task.abort_handle())?;

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
    let subscription = state.run_manager.subscribe(&run_id, query.after)?;

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
                        match state.manager.events_after(&state.run_id, state.after) {
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
                        match state.manager.events_after(&state.run_id, state.after) {
                            Ok(events) if !events.is_empty() => state.pending.extend(events),
                            _ => return None,
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
}

impl RuntimeEventSink for PersistentRunEventSink {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.manager.publish(
            &self.run_id,
            RunEvent::AssistantMessageSaved {
                message_id: message_id.as_str().to_string(),
            },
        ) {
            log_api_error(&error);
        }
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.manager.publish(
            &self.run_id,
            RunEvent::ToolResultSaved {
                message_id: message_id.as_str().to_string(),
            },
        ) {
            log_api_error(&error);
        }
    }
}

/// Persists live display deltas so a reloaded UI can replay the active output.
struct PersistentRunOutput {
    manager: Arc<RunManager>,
    run_id: String,
}

impl RuntimeOutput for PersistentRunOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.manager.publish(
            &self.run_id,
            RunEvent::AssistantDelta {
                text: text.to_string(),
            },
        )?;
        Ok(())
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.manager.publish(
            &self.run_id,
            RunEvent::ReasoningDelta {
                text: text.to_string(),
            },
        )?;
        Ok(())
    }

    fn tool_call_delta(
        &self,
        index: u16,
        id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Result<()> {
        self.manager.publish(
            &self.run_id,
            RunEvent::ToolCallDelta {
                index,
                id: id.map(str::to_string),
                name: name.map(str::to_string),
                arguments_delta: arguments_delta.map(str::to_string),
            },
        )?;
        Ok(())
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[ToolCall]) {}
}
