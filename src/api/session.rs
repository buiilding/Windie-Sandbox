//! Session lifecycle and event API route handlers.

use super::*;

#[derive(Debug, Deserialize)]
/// Request body for starting a backend-owned session.
pub(super) struct CreateSessionRequest {
    pub(super) head_message_id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) reasoning: Option<ReasoningRequest>,
}

impl CreateSessionRequest {
    fn reasoning(&self) -> Option<ReasoningRequest> {
        self.reasoning
            .clone()
            .filter(|reasoning| !reasoning.is_empty())
    }
}

#[derive(Debug, Serialize)]
/// Serializable run response.
pub(super) struct SessionResponse {
    pub(super) id: String,
    pub(super) conversation_id: String,
    pub(super) start_head_message_id: Option<String>,
    pub(super) current_head_message_id: Option<String>,
    pub(super) status: SessionStatus,
    pub(super) model: String,
    pub(super) reasoning: Option<ReasoningRequest>,
    pub(super) error: Option<String>,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

impl SessionResponse {
    pub(super) fn from_session(session: Session) -> Self {
        Self {
            id: session.id.as_str().to_string(),
            conversation_id: session.conversation_id.as_str().to_string(),
            start_head_message_id: session
                .start_head_message_id
                .map(|id| id.as_str().to_string()),
            current_head_message_id: session
                .current_head_message_id
                .map(|id| id.as_str().to_string()),
            status: session.status,
            model: session.model,
            reasoning: session.reasoning,
            error: session.error,
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
/// List of runtime sessions visible to clients.
pub(super) struct SessionListResponse {
    pub(super) sessions: Vec<SessionResponse>,
}

/// Starts a backend-owned session from an explicit conversation head.
pub(super) async fn create_session(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<CreateSessionRequest>,
) -> ApiResult<SessionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = request.head_message_id.clone().map(MessageId::new);
    let reasoning = request.reasoning();
    let session = state
        .session_manager
        .start_continue_wakeup(ContinueWakeup {
            conversation_id,
            head_message_id,
            model: request.model.map(ModelName::new),
            reasoning,
        })?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Lists persisted sessions.
pub(super) async fn list_sessions(State(state): State<ApiState>) -> ApiResult<SessionListResponse> {
    let store = open_store(&state)?;
    let sessions = store
        .list_sessions()?
        .into_iter()
        .map(SessionResponse::from_session)
        .collect();

    Ok(Json(SessionListResponse { sessions }))
}

/// Loads one persisted session.
pub(super) async fn get_run(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let store = open_store(&state)?;
    let session = store.load_session(&SessionId::new(session_id))?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Stops one live session explicitly.
pub(super) async fn stop_run(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let session_id = SessionId::new(session_id);
    state.session_manager.stop(&session_id)?;
    let store = open_store(&state)?;
    let session = store.load_session(&session_id)?;

    Ok(Json(SessionResponse::from_session(session)))
}

#[derive(Debug, Deserialize)]
/// Cursor query for replaying session events.
pub(super) struct SessionEventsQuery {
    pub(super) after: Option<i64>,
}

/// Streams persisted and live events for one session.
pub(super) async fn session_events(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    let session_id = SessionId::new(session_id);
    let store = open_store(&state)?;
    let replay = store.load_session_events_after(&session_id, query.after)?;
    let subscription = state.session_manager.subscribe(&session_id);
    let stream = stream::unfold(
        SessionSseState {
            replay: replay.into(),
            subscription,
        },
        |mut state| async move {
            let record = if let Some(record) = state.replay.pop_front() {
                record
            } else {
                let subscription = state.subscription.as_mut()?;
                match subscription.recv().await {
                    Ok(record) => record,
                    Err(_) => return None,
                }
            };
            let event_name = record.event.event_name();
            let data = session_event_data(&record);
            let sse = Event::default()
                .id(record.id.to_string())
                .event(event_name)
                .data(data);

            Some((Ok::<Event, Infallible>(sse), state))
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
