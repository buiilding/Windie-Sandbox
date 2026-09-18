//! HTTP and SSE interface for account-owned hosted conversations.

use std::convert::Infallible;

use axum::{
    Json, Router,
    extract::{Extension, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header::IF_MATCH},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

use super::conversation::{
    AppendHostedMessage, CreateHostedConversation, ForkHostedConversation,
    HostedConversationOperations, RemoveHostedMessage, TruncateHostedConversation,
    UpdateHostedMessage, mutation_body,
};

use super::{
    HostedConfig, HostedRuntime, HostedStore,
    account::HostedAccount,
    auth::{HostedAuth, HostedAuthError},
    events,
    store::{HostedMessagePartInput, HostedStoreError},
};
use crate::session::SessionEventHub;

/// Shared hosted request state. It contains no user-derived account ID.
#[derive(Clone)]
struct HostedApiState {
    store: HostedStore,
    conversations: HostedConversationOperations,
    runtime: HostedRuntime,
    live_events: SessionEventHub,
    auth: HostedAuth,
}

/// Starts the hosted API only after its caller has migrated its private DB.
pub async fn serve(config: HostedConfig, store: HostedStore) -> anyhow::Result<()> {
    let allowed_origin = HeaderValue::from_str(&config.allowed_origin)
        .map_err(|error| anyhow::anyhow!("invalid WINDIE_HOSTED_ALLOWED_ORIGIN: {error}"))?;
    let live_events = SessionEventHub::default();
    events::start_session_event_listener(store.clone(), live_events.clone());
    let runtime = HostedRuntime::new(
        store.clone(),
        config.bifrost_base_url.clone(),
        live_events.clone(),
    );
    let recovered = runtime.recover_interrupted_sessions().await?;
    if recovered > 0 {
        eprintln!("marked {recovered} interrupted hosted session(s) failed after restart");
    }
    runtime.start_wakeup_scheduler();
    let state = HostedApiState {
        conversations: HostedConversationOperations::new(
            store.clone(),
            config.default_model.clone(),
        ),
        runtime,
        live_events,
        store,
        auth: HostedAuth::new(config.supabase_url, config.supabase_publishable_key),
    };
    let app = router(state, allowed_origin);
    let listener = TcpListener::bind(config.address).await.map_err(|error| {
        anyhow::anyhow!(
            "failed to bind hosted server at {}: {error}",
            config.address
        )
    })?;
    axum::serve(listener, app)
        .await
        .map_err(|error| anyhow::anyhow!("hosted server failed: {error}"))
}

fn router(state: HostedApiState, allowed_origin: HeaderValue) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(allowed_origin)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            IF_MATCH,
            HeaderNameExt::idempotency_key(),
        ]);
    Router::new()
        .route("/health", get(health))
        .route(
            "/v1/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route("/v1/conversations/{conversation_id}", get(get_conversation))
        .route(
            "/v1/conversations/{conversation_id}/messages",
            post(append_message),
        )
        .route(
            "/v1/conversations/{conversation_id}/messages/{message_id}",
            patch(update_message).delete(remove_message),
        )
        .route(
            "/v1/conversations/{conversation_id}/truncate",
            post(truncate_conversation),
        )
        .route(
            "/v1/conversations/{conversation_id}/fork",
            post(fork_conversation),
        )
        .route("/v1/events", get(events_after))
        .route(
            "/v1/conversations/{conversation_id}/sessions/resolve",
            post(resolve_or_create_session),
        )
        .route(
            "/v1/conversations/{conversation_id}/query",
            post(query_conversation),
        )
        .route(
            "/v1/conversations/{conversation_id}/continue",
            post(continue_conversation),
        )
        .route("/v1/sessions/{session_id}", get(get_session))
        .route("/v1/sessions/{session_id}/events", get(session_events))
        .route("/v1/sessions/{session_id}/stop", post(stop_session))
        .route("/v1/sessions/{session_id}/wakeup", post(schedule_wakeup))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate_request,
        ))
        .layer(cors)
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "windie-server",
    })
}

/// Authenticates every non-health request and resolves its internal account ID
/// once. All handlers receive that account through request extensions.
async fn authenticate_request(
    State(state): State<HostedApiState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS || request.uri().path() == "/health" {
        return next.run(request).await;
    }
    let subject = match state.auth.authenticate(request.headers()).await {
        Ok(subject) => subject,
        Err(error) => return HostedApiError::auth(error).into_response(),
    };
    let account = match state.store.resolve_account(&subject.0).await {
        Ok(account) => account,
        Err(error) => return HostedApiError::store(error).into_response(),
    };
    request.extensions_mut().insert(account);
    next.run(request).await
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<i64>,
}

#[derive(Debug)]
struct HostedApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl HostedApiError {
    fn auth(error: HostedAuthError) -> Self {
        let status = match error {
            HostedAuthError::MissingBearerToken | HostedAuthError::InvalidBearerToken => {
                StatusCode::UNAUTHORIZED
            }
            HostedAuthError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        };
        Self {
            status,
            body: ErrorBody {
                error: "authentication_failed",
                message: error.to_string(),
                expected_revision: None,
                current_revision: None,
            },
        }
    }

    fn store(error: HostedStoreError) -> Self {
        match error {
            HostedStoreError::NotFound
            | HostedStoreError::InvalidHead
            | HostedStoreError::SessionNotFound => Self {
                status: StatusCode::NOT_FOUND,
                body: ErrorBody {
                    error: "not_found",
                    message: error.to_string(),
                    expected_revision: None,
                    current_revision: None,
                },
            },
            HostedStoreError::StaleRevision { expected, current } => Self {
                status: StatusCode::CONFLICT,
                body: ErrorBody {
                    error: "stale_revision",
                    message: format!(
                        "conversation revision is stale (expected {expected}, current {current})"
                    ),
                    expected_revision: Some(expected),
                    current_revision: Some(current),
                },
            },
            HostedStoreError::AmbiguousSession | HostedStoreError::SessionConflict => Self {
                status: StatusCode::CONFLICT,
                body: ErrorBody {
                    error: "session_conflict",
                    message: error.to_string(),
                    expected_revision: None,
                    current_revision: None,
                },
            },
            HostedStoreError::Database(_) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: ErrorBody {
                    error: "storage_failed",
                    message: "hosted storage is temporarily unavailable".to_string(),
                    expected_revision: None,
                    current_revision: None,
                },
            },
            other => Self {
                status: StatusCode::BAD_REQUEST,
                body: ErrorBody {
                    error: "invalid_request",
                    message: other.to_string(),
                    expected_revision: None,
                    current_revision: None,
                },
            },
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ErrorBody {
                error: "invalid_request",
                message: message.into(),
                expected_revision: None,
                current_revision: None,
            },
        }
    }
}

impl IntoResponse for HostedApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct CreateConversationRequest {
    model: Option<String>,
}

async fn create_conversation(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    headers: HeaderMap,
    request: Option<Json<CreateConversationRequest>>,
) -> Result<Response, HostedApiError> {
    let command = CreateHostedConversation {
        model: request.and_then(|Json(request)| request.model),
        idempotency_key: idempotency_key(&headers)?.to_string(),
    };
    let response = state
        .conversations
        .create(&account, command)
        .await
        .map_err(HostedApiError::store)?;
    Ok(mutation_response(response))
}

#[derive(Debug, Deserialize)]
struct ConversationQuery {
    head: Option<String>,
}

async fn list_conversations(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
) -> Result<Json<Value>, HostedApiError> {
    let (conversations, event_cursor) = state
        .conversations
        .list(&account)
        .await
        .map_err(HostedApiError::store)?;
    Ok(Json(
        json!({"conversations": conversations, "event_cursor": event_cursor}),
    ))
}

async fn get_conversation(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ConversationQuery>,
) -> Result<Json<Value>, HostedApiError> {
    let conversation = state
        .conversations
        .load(&account, &conversation_id, query.head.as_deref())
        .await
        .map_err(HostedApiError::store)?;
    Ok(Json(json!({"conversation": conversation})))
}

#[derive(Debug, Deserialize)]
struct AppendMessageRequest {
    parent_message_id: Option<String>,
    role: String,
    text: Option<String>,
    #[serde(default)]
    parts: Vec<HostedMessagePartInput>,
}

async fn append_message(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<AppendMessageRequest>,
) -> Result<Response, HostedApiError> {
    let command = AppendHostedMessage {
        conversation_id,
        expected_revision: expected_revision(&headers)?,
        parent_message_id: request.parent_message_id,
        role: request.role,
        text: request.text,
        parts: request.parts,
        idempotency_key: idempotency_key(&headers)?.to_string(),
    };
    let response = state
        .conversations
        .append(&account, command)
        .await
        .map_err(HostedApiError::store)?;
    Ok(mutation_response(response))
}

#[derive(Debug, Deserialize)]
struct UpdateMessageRequest {
    text: String,
}

async fn update_message(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path((conversation_id, message_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<UpdateMessageRequest>,
) -> Result<Response, HostedApiError> {
    let command = UpdateHostedMessage {
        conversation_id,
        message_id,
        expected_revision: expected_revision(&headers)?,
        text: request.text,
        idempotency_key: idempotency_key(&headers)?.to_string(),
    };
    let response = state
        .conversations
        .update(&account, command)
        .await
        .map_err(HostedApiError::store)?;
    Ok(mutation_response(response))
}

async fn remove_message(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path((conversation_id, message_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, HostedApiError> {
    let command = RemoveHostedMessage {
        conversation_id,
        message_id,
        expected_revision: expected_revision(&headers)?,
        idempotency_key: idempotency_key(&headers)?.to_string(),
    };
    let response = state
        .conversations
        .remove(&account, command)
        .await
        .map_err(HostedApiError::store)?;
    Ok(mutation_response(response))
}

#[derive(Debug, Deserialize)]
struct MessageIdRequest {
    message_id: String,
}

async fn truncate_conversation(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<MessageIdRequest>,
) -> Result<Response, HostedApiError> {
    let command = TruncateHostedConversation {
        conversation_id,
        message_id: request.message_id,
        expected_revision: expected_revision(&headers)?,
        idempotency_key: idempotency_key(&headers)?.to_string(),
    };
    let response = state
        .conversations
        .truncate(&account, command)
        .await
        .map_err(HostedApiError::store)?;
    Ok(mutation_response(response))
}

async fn fork_conversation(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<MessageIdRequest>,
) -> Result<Response, HostedApiError> {
    let command = ForkHostedConversation {
        conversation_id,
        message_id: request.message_id,
        expected_revision: expected_revision(&headers)?,
        idempotency_key: idempotency_key(&headers)?.to_string(),
    };
    let response = state
        .conversations
        .fork(&account, command)
        .await
        .map_err(HostedApiError::store)?;
    Ok(mutation_response(response))
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<i64>,
}

async fn events_after(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    Sse::new(events::account_events(
        state.conversations,
        account,
        query.after.unwrap_or(0),
    ))
    .keep_alive(KeepAlive::default())
}

/// Session selection is backend-owned: an existing unique branch is reused,
/// zero matches create one, and multiple matches return an explicit conflict.
#[derive(Debug, Deserialize)]
struct ResolveSessionRequest {
    head_message_id: Option<String>,
    reasoning: Option<crate::llm::ReasoningRequest>,
}

async fn resolve_or_create_session(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    request: Option<Json<ResolveSessionRequest>>,
) -> Result<Json<Value>, HostedApiError> {
    let request = request
        .map(|Json(request)| request)
        .unwrap_or(ResolveSessionRequest {
            head_message_id: None,
            reasoning: None,
        });
    let session = state
        .store
        .resolve_or_create_session(
            &account,
            &conversation_id,
            request.head_message_id.as_deref(),
            request.reasoning,
        )
        .await
        .map_err(HostedApiError::store)?;
    session_response(&state.store, &account, session).await
}

#[derive(Debug, Deserialize)]
struct HostedQueryRequest {
    head_message_id: Option<String>,
    text: Option<String>,
    #[serde(default)]
    parts: Vec<HostedMessagePartInput>,
    reasoning: Option<crate::llm::ReasoningRequest>,
}

async fn query_conversation(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    Json(request): Json<HostedQueryRequest>,
) -> Result<Json<Value>, HostedApiError> {
    let result = state
        .runtime
        .query_conversation(
            account.clone(),
            &conversation_id,
            request.head_message_id.as_deref(),
            super::conversation::message_parts(request.text, request.parts),
            request.reasoning,
        )
        .await
        .map_err(HostedApiError::store)?;
    let queue_depth = state
        .store
        .session_input_count(&account, &result.session.id)
        .await
        .map_err(HostedApiError::store)?;
    Ok(Json(json!({
        "session": result.session,
        "queued": result.queued,
        "queue_depth": queue_depth,
        "queue_id": result.input_id.map(|id| id.as_str().to_string()),
    })))
}

#[derive(Debug, Deserialize)]
struct ContinueSessionRequest {
    head_message_id: Option<String>,
    reasoning: Option<crate::llm::ReasoningRequest>,
}

async fn continue_conversation(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(conversation_id): Path<String>,
    request: Option<Json<ContinueSessionRequest>>,
) -> Result<Json<Value>, HostedApiError> {
    let request = request
        .map(|Json(request)| request)
        .unwrap_or(ContinueSessionRequest {
            head_message_id: None,
            reasoning: None,
        });
    let session = state
        .store
        .resolve_or_create_session(
            &account,
            &conversation_id,
            request.head_message_id.as_deref(),
            request.reasoning,
        )
        .await
        .map_err(HostedApiError::store)?;
    let session = state
        .runtime
        .continue_session(account.clone(), &session.id)
        .await
        .map_err(HostedApiError::store)?;
    session_response(&state.store, &account, session).await
}

async fn get_session(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, HostedApiError> {
    let session = state
        .store
        .session(&account, &crate::session::SessionId::new(session_id))
        .await
        .map_err(HostedApiError::store)?;
    session_response(&state.store, &account, session).await
}

async fn stop_session(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, HostedApiError> {
    let session_id = crate::session::SessionId::new(session_id);
    let session = state
        .runtime
        .stop(&account, &session_id)
        .await
        .map_err(HostedApiError::store)?;
    session_response(&state.store, &account, session).await
}

#[derive(Debug, Deserialize)]
struct ScheduleWakeupRequest {
    trigger_type: String,
    due_at_millis: i64,
}

async fn schedule_wakeup(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(session_id): Path<String>,
    Json(request): Json<ScheduleWakeupRequest>,
) -> Result<Json<Value>, HostedApiError> {
    let session_id = crate::session::SessionId::new(session_id);
    let wakeup_id = state
        .store
        .schedule_wakeup(
            &account,
            &session_id,
            &request.trigger_type,
            request.due_at_millis,
        )
        .await
        .map_err(HostedApiError::store)?;
    Ok(Json(
        json!({"wakeup_id": wakeup_id, "session_id": session_id.as_str()}),
    ))
}

#[derive(Debug, Deserialize)]
struct SessionEventsQuery {
    after: Option<i64>,
}

async fn session_events(
    State(state): State<HostedApiState>,
    Extension(account): Extension<HostedAccount>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    HostedApiError,
> {
    let session_id = crate::session::SessionId::new(session_id);
    // Check ownership before returning a long-lived stream. The stream repeats
    // that account constraint on every durable poll.
    state
        .store
        .session(&account, &session_id)
        .await
        .map_err(HostedApiError::store)?;
    Ok(Sse::new(events::session_events(
        state.store,
        account,
        session_id,
        query.after.unwrap_or(0),
        state.live_events,
    ))
    .keep_alive(KeepAlive::default()))
}

async fn session_response(
    store: &HostedStore,
    account: &HostedAccount,
    session: crate::session::Session,
) -> Result<Json<Value>, HostedApiError> {
    let queue_depth = store
        .session_input_count(account, &session.id)
        .await
        .map_err(HostedApiError::store)?;
    let event_cursor = store
        .session_event_cursor(account, &session.id)
        .await
        .map_err(HostedApiError::store)?;
    Ok(Json(json!({
        "session": session,
        "queue_depth": queue_depth,
        "event_cursor": event_cursor,
    })))
}

fn mutation_response(response: crate::hosted::MutationResponse) -> Response {
    let (status, body) = mutation_body(response);
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(body)).into_response()
}

fn idempotency_key(headers: &HeaderMap) -> Result<&str, HostedApiError> {
    headers
        .get(HeaderNameExt::idempotency_key())
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| HostedApiError::invalid("Idempotency-Key header is required"))
}

fn expected_revision(headers: &HeaderMap) -> Result<i64, HostedApiError> {
    let value = headers
        .get(IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            HostedApiError::invalid("If-Match conversation revision header is required")
        })?;
    let value = value.trim().trim_matches('"');
    value
        .parse::<i64>()
        .ok()
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| {
            HostedApiError::invalid("If-Match must be a non-negative conversation revision")
        })
}

/// Keeps the non-standard idempotency header spelled in one place.
struct HeaderNameExt;

impl HeaderNameExt {
    fn idempotency_key() -> axum::http::HeaderName {
        axum::http::HeaderName::from_static("idempotency-key")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_if_match_revision() {
        let mut headers = HeaderMap::new();
        headers.insert(IF_MATCH, HeaderValue::from_static("\"42\""));
        assert_eq!(expected_revision(&headers).unwrap(), 42);
    }

    #[test]
    fn rejects_missing_idempotency_key() {
        assert!(idempotency_key(&HeaderMap::new()).is_err());
    }
}
