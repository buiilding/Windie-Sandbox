//! Local API route table and HTTP middleware wiring.

use super::*;

/// Builds the route table for the local API surface.
///
/// Handlers translate HTTP requests into shared operations and map returned
/// values into JSON responses. The router only owns HTTP mapping.
pub(super) fn router(state: ApiState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://localhost:3000"),
            HeaderValue::from_static("http://127.0.0.1:3000"),
            HeaderValue::from_static("http://localhost:5173"),
            HeaderValue::from_static("http://127.0.0.1:5173"),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([CONTENT_TYPE, HeaderName::from_static(API_TOKEN_HEADER)]);

    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route("/api/models", get(list_models))
        .route("/api/model-parameters", get(model_parameters))
        .route("/api/tools", get(list_tools))
        .route("/api/tools/{provider_id}", get(list_provider_tools))
        .route("/api/gateway/start", post(start_gateway))
        .route("/api/gateway/stop", post(stop_gateway))
        .route(
            "/api/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(inspect_conversation).delete(remove_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/messages",
            post(insert_message),
        )
        .route(
            "/api/conversations/{conversation_id}/messages/{message_id}",
            patch(update_message).delete(remove_message),
        )
        .route(
            "/api/conversations/{conversation_id}/images/{asset_id}",
            get(get_conversation_image),
        )
        .route(
            "/api/conversations/{conversation_id}/system-prompt",
            patch(set_system_prompt).delete(remove_system_prompt),
        )
        .route(
            "/api/conversations/{conversation_id}/model",
            patch(set_conversation_model),
        )
        .route(
            "/api/conversations/{conversation_id}/reasoning",
            patch(set_conversation_reasoning),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-approval-mode",
            patch(set_tool_approval_mode),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-schemas",
            post(insert_tool_schema),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-schemas/{name}",
            patch(update_tool_schema).delete(remove_tool_schema),
        )
        .route(
            "/api/conversations/{conversation_id}/tools",
            get(list_attached_tools).post(attach_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/tools/batch",
            post(attach_tools),
        )
        .route(
            "/api/conversations/{conversation_id}/tools/{schema_name}",
            axum::routing::delete(detach_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/truncate",
            post(truncate_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/fork",
            post(fork_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/run-approvals",
            get(list_conversation_session_approvals),
        )
        .route(
            "/api/conversations/{conversation_id}/sessions",
            post(create_session),
        )
        .route(
            "/api/conversations/{conversation_id}/wakeups/continue",
            post(create_session),
        )
        .route(
            "/api/conversations/{conversation_id}/wakeups/query",
            post(create_session),
        )
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session_id}", get(get_run))
        .route(
            "/api/sessions/{session_id}/approvals",
            get(list_session_approvals),
        )
        .route("/api/sessions/{session_id}/events", get(session_events))
        .route("/api/sessions/{session_id}/stop", post(stop_run))
        .route(
            "/api/sessions/{session_id}/approvals/{tool_call_id}/approve",
            post(approve_session_tool),
        )
        .route(
            "/api/sessions/{session_id}/approvals/{tool_call_id}/deny",
            post(deny_session_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/runs",
            post(create_session),
        )
        .route("/api/runs", get(list_sessions))
        .route("/api/runs/{session_id}", get(get_run))
        .route(
            "/api/runs/{session_id}/approvals",
            get(list_session_approvals),
        )
        .route("/api/runs/{session_id}/events", get(session_events))
        .route("/api/runs/{session_id}/stop", post(stop_run))
        .route(
            "/api/runs/{session_id}/approvals/{tool_call_id}/approve",
            post(approve_session_tool),
        )
        .route(
            "/api/runs/{session_id}/approvals/{tool_call_id}/deny",
            post(deny_session_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/input-tokens",
            post(count_input_tokens),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_token,
        ))
        .layer(DefaultBodyLimit::max(API_JSON_BODY_LIMIT_BYTES))
        .layer(cors)
        .with_state(state)
}
