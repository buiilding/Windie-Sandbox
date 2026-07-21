//! Session lifecycle and event API route handlers.

use super::*;

#[derive(Debug, Deserialize)]
/// Request body for creating a selectable session branch.
pub(super) struct CreateSessionBranchRequest {
    pub(super) head_message_id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) reasoning: Option<ReasoningRequest>,
}

impl CreateSessionBranchRequest {
    fn reasoning(&self) -> Option<ReasoningRequest> {
        self.reasoning
            .clone()
            .filter(|reasoning| !reasoning.is_empty())
    }
}

#[derive(Debug, Deserialize)]
/// One user query to append to a selected session branch.
pub(super) struct SessionQueryRequest {
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) parts: Vec<InsertMessagePart>,
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

/// Creates a selectable session branch without starting model execution.
pub(super) async fn create_session_branch(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<CreateSessionBranchRequest>,
) -> ApiResult<SessionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = request.head_message_id.clone().map(MessageId::new);
    let model = match request.model.clone() {
        Some(model) => model,
        None => {
            let store = open_store(&state)?;
            operation::conversation_model(&store, &conversation_id)?
                .as_str()
                .to_string()
        }
    };
    let session = state.session_manager.create_session_branch(
        conversation_id,
        head_message_id,
        model,
        request.reasoning(),
    )?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Lists all selectable sessions belonging to one conversation.
pub(super) async fn list_conversation_sessions(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<SessionListResponse> {
    let store = open_store(&state)?;
    let sessions = store
        .list_conversation_sessions(&ConversationId::new(conversation_id))?
        .into_iter()
        .map(SessionResponse::from_session)
        .collect();

    Ok(Json(SessionListResponse { sessions }))
}

/// Appends a user message to one session branch and starts its runtime.
pub(super) async fn query_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Json(request): Json<SessionQueryRequest>,
) -> ApiResult<SessionResponse> {
    let parts = normalize_insert_parts(request.text, request.parts)?;
    let session = state
        .session_manager
        .query_session(&SessionId::new(session_id), &parts)?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Continues one selected session from its current head.
pub(super) async fn continue_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let session = state
        .session_manager
        .continue_session(&SessionId::new(session_id))?;

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

/// Removes one terminal session and its exclusive conversation-tree suffix.
pub(super) async fn remove_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<DeletedResponse> {
    state
        .session_manager
        .remove_session(&SessionId::new(session_id))
        .await?;

    Ok(Json(DeletedResponse { deleted: true }))
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
