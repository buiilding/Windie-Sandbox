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
    pub(super) head_message_id: Option<String>,
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) parts: Vec<InsertMessagePart>,
}

#[derive(Debug, Deserialize)]
/// A conversation-head target for resolution and continue operations.
pub(super) struct SessionHeadRequest {
    pub(super) head_message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for changing one session's autonomous idle-wakeup setting.
pub(super) struct SetKeepAwakeRequest {
    pub(super) keep_awake: bool,
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
    pub(super) keep_awake: bool,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) queued: bool,
    pub(super) queue_depth: usize,
    pub(super) queue_id: Option<String>,
    pub(super) latest_event_id: Option<i64>,
    pub(super) node_count: usize,
    pub(super) protected_message_ids: Vec<String>,
    pub(super) deletion_allowed: bool,
}

impl SessionResponse {
    pub(super) fn from_session(session: Session, node_count: usize) -> Self {
        Self::from_session_with_queue(session, 0, node_count)
    }

    pub(super) fn from_session_with_queue(
        session: Session,
        queue_depth: usize,
        node_count: usize,
    ) -> Self {
        let deletion_allowed = !matches!(
            session.status,
            SessionStatus::Running | SessionStatus::WaitingForApproval
        );
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
            keep_awake: session.keep_awake,
            created_at: session.created_at,
            updated_at: session.updated_at,
            queued: false,
            queue_depth,
            queue_id: None,
            latest_event_id: None,
            node_count,
            protected_message_ids: Vec::new(),
            deletion_allowed,
        }
    }

    fn from_query(
        result: crate::session::SessionQueryResult,
        latest_event_id: Option<i64>,
        node_count: usize,
    ) -> Self {
        let mut response = Self::from_session(result.session, node_count);
        response.queued = result.queued;
        response.queue_depth = result.queue_depth;
        response.queue_id = result.input_id.map(|id| id.as_str().to_string());
        response.latest_event_id = latest_event_id;
        response
    }
}

/// Persists whether one session should wake after the user has been idle.
pub(super) async fn set_session_keep_awake(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Json(request): Json<SetKeepAwakeRequest>,
) -> ApiResult<SessionResponse> {
    let session = state
        .session_manager
        .set_keep_awake(&SessionId::new(session_id), request.keep_awake)?;
    let store = open_store(&state)?;
    Ok(Json(response_with_queue(&store, session)?))
}

pub(super) fn response_with_queue(store: &Store, session: Session) -> Result<SessionResponse> {
    let protected_message_ids = store.protected_message_ids_for_session(&session)?;
    let queue_depth = store.session_input_count(&session.id)?;
    let latest_event_id = store.latest_session_event_id(&session.id)?;
    let node_count = store.session_node_count(&session)?;
    let mut response = SessionResponse::from_session_with_queue(session, queue_depth, node_count);
    response.latest_event_id = latest_event_id;
    response.protected_message_ids = protected_message_ids;
    Ok(response)
}

fn response_from_query(
    store: &Store,
    result: crate::session::SessionQueryResult,
    latest_event_id: Option<i64>,
    node_count: usize,
) -> Result<SessionResponse> {
    let protected_message_ids = store.protected_message_ids_for_session(&result.session)?;
    let mut response = SessionResponse::from_query(result, latest_event_id, node_count);
    response.protected_message_ids = protected_message_ids;
    Ok(response)
}

#[derive(Debug, Serialize)]
/// List of runtime sessions visible to clients.
pub(super) struct SessionListResponse {
    pub(super) sessions: Vec<SessionResponse>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Backend-owned result for resolving a conversation head to a session branch.
pub(super) enum SessionResolutionResponse {
    ExistingSession { session: Box<SessionResponse> },
    NoSessionAtHead,
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

    let store = open_store(&state)?;
    Ok(Json(response_with_queue(&store, session)?))
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
        .map(|session| response_with_queue(&store, session))
        .collect::<Result<Vec<_>>>()?;

    Ok(Json(SessionListResponse { sessions }))
}

/// Resolves one conversation head to its durable session branch, if any.
pub(super) async fn resolve_session_at_head(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SessionHeadRequest>,
) -> ApiResult<SessionResolutionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = request.head_message_id.map(MessageId::new);
    let resolution = state
        .session_manager
        .resolve_session_at_head(&conversation_id, head_message_id.as_ref())?;
    let response = match resolution {
        crate::session::SessionResolution::Existing(session) => {
            let store = open_store(&state)?;
            SessionResolutionResponse::ExistingSession {
                session: Box::new(response_with_queue(&store, *session)?),
            }
        }
        crate::session::SessionResolution::NoSessionAtHead => {
            SessionResolutionResponse::NoSessionAtHead
        }
        crate::session::SessionResolution::Ambiguous(sessions) => {
            let ids = sessions
                .into_iter()
                .map(|session| session.id.as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(crate::error::conflict(format!(
                "multiple sessions exist at conversation head: {ids}"
            ))
            .into());
        }
    };

    Ok(Json(response))
}

/// Appends a user message to one session branch and starts its runtime.
pub(super) async fn query_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Json(request): Json<SessionQueryRequest>,
) -> ApiResult<SessionResponse> {
    let parts = normalize_insert_parts(request.text, request.parts)?;
    let result = state
        .session_manager
        .query_session(&SessionId::new(session_id), &parts)?;
    let store = open_store(&state)?;
    let latest_event_id = store.latest_session_event_id(&result.session.id)?;
    let node_count = store.session_node_count(&result.session)?;

    Ok(Json(response_from_query(
        &store,
        result,
        latest_event_id,
        node_count,
    )?))
}

/// Resolves or creates a session at a conversation head, then appends input.
pub(super) async fn query_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SessionQueryRequest>,
) -> ApiResult<SessionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = request.head_message_id.map(MessageId::new);
    let parts = normalize_insert_parts(request.text, request.parts)?;
    let result = state.session_manager.query_conversation_at_head(
        &conversation_id,
        head_message_id.as_ref(),
        &parts,
    )?;
    let store = open_store(&state)?;
    let latest_event_id = store.latest_session_event_id(&result.session.id)?;
    let node_count = store.session_node_count(&result.session)?;

    Ok(Json(response_from_query(
        &store,
        result,
        latest_event_id,
        node_count,
    )?))
}

/// Continues one selected session from its current head.
pub(super) async fn continue_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let session = state
        .session_manager
        .continue_session(&SessionId::new(session_id))?;

    let store = open_store(&state)?;
    Ok(Json(response_with_queue(&store, session)?))
}

/// Resolves or creates a session at a conversation head, then continues it.
pub(super) async fn continue_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SessionHeadRequest>,
) -> ApiResult<SessionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = request.head_message_id.map(MessageId::new);
    let session = state
        .session_manager
        .continue_conversation_at_head(&conversation_id, head_message_id.as_ref())?;
    let store = open_store(&state)?;
    Ok(Json(response_with_queue(&store, session)?))
}

/// Lists persisted sessions.
pub(super) async fn list_sessions(State(state): State<ApiState>) -> ApiResult<SessionListResponse> {
    let store = open_store(&state)?;
    let sessions = store
        .list_sessions()?
        .into_iter()
        .map(|session| response_with_queue(&store, session))
        .collect::<Result<Vec<_>>>()?;

    Ok(Json(SessionListResponse { sessions }))
}

/// Loads one persisted session.
pub(super) async fn get_run(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let store = open_store(&state)?;
    let session = store.load_session(&SessionId::new(session_id))?;
    Ok(Json(response_with_queue(&store, session)?))
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

    Ok(Json(response_with_queue(&store, session)?))
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
            store_path: state.store_path.clone(),
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
            let data = session_event_data(state.store_path.as_deref(), &record);
            let sse = Event::default()
                .id(record.id.to_string())
                .event(event_name)
                .data(data);

            Some((Ok::<Event, Infallible>(sse), state))
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
