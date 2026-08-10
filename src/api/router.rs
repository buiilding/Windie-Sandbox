//! Local API route table and HTTP middleware wiring.

use super::*;

/// Builds the route table for the local API surface.
///
/// Handlers translate HTTP requests into shared operations and map returned
/// values into JSON responses. The router only owns HTTP mapping.
pub(super) fn router(state: ApiState) -> Router {
    Router::new().merge(api_router(state))
}

/// Builds the unauthenticated localhost `/api/*` route table.
///
/// CORS stays scoped to the API so the standalone Inspector and browser clients
/// served from webpack dev servers (ports 3000/5173) can call localhost.
fn api_router(state: ApiState) -> Router {
    let mut origins = vec![
        HeaderValue::from_static("http://localhost:3000"),
        HeaderValue::from_static("http://127.0.0.1:3000"),
        HeaderValue::from_static("http://localhost:5173"),
        HeaderValue::from_static("http://127.0.0.1:5173"),
    ];
    if let Ok(origin) =
        HeaderValue::try_from(format!("http://{}", crate::config::inspector_address()))
    {
        origins.push(origin);
    }

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([CONTENT_TYPE]);

    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route("/api/shutdown", post(shutdown))
        .route("/api/models", get(list_models))
        .route("/api/llm/providers", get(list_provider_catalog))
        .route(
            "/api/llm/providers/{provider}/keys",
            get(list_provider_keys).post(create_provider_key),
        )
        .route(
            "/api/llm/providers/{provider}/ensure",
            post(ensure_provider),
        )
        .route(
            "/api/llm/providers/{provider}/keys/{key_id}",
            axum::routing::delete(delete_provider_key),
        )
        .route("/api/env", axum::routing::put(set_env_values_handler))
        .route("/api/model-parameters", get(model_parameters))
        .route("/api/tools", get(list_tools))
        .route("/api/tools/{provider_id}", get(list_provider_tools))
        .route("/api/providers", get(list_providers))
        .route(
            "/api/providers/chrome-devtools/remote-debugging",
            get(chrome_devtools_remote_debugging),
        )
        .route(
            "/api/providers/{provider_id}",
            get(get_provider).delete(uninstall_provider),
        )
        .route(
            "/api/providers/{provider_id}/install",
            post(install_provider),
        )
        .route("/api/providers/{provider_id}/setup", post(setup_provider))
        .route(
            "/api/providers/{provider_id}/configuration",
            post(configure_provider),
        )
        .route("/api/providers/{provider_id}/enable", post(enable_provider))
        .route(
            "/api/providers/{provider_id}/disable",
            post(disable_provider),
        )
        .route("/api/providers/{provider_id}/repair", post(repair_provider))
        .route(
            "/api/providers/{provider_id}/health-check",
            post(health_check_provider),
        )
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
            get(list_conversation_sessions).post(create_session_branch),
        )
        .route(
            "/api/conversations/{conversation_id}/sessions/resolve",
            post(resolve_session_at_head),
        )
        .route(
            "/api/conversations/{conversation_id}/query",
            post(query_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/continue",
            post(continue_conversation),
        )
        .route("/api/sessions/{session_id}/query", post(query_session))
        .route(
            "/api/sessions/{session_id}/continue",
            post(continue_session),
        )
        .route("/api/sessions", get(list_sessions))
        .route(
            "/api/sessions/{session_id}",
            get(get_run).delete(remove_session),
        )
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
            "/api/conversations/{conversation_id}/input-tokens",
            post(count_input_tokens),
        )
        .layer(DefaultBodyLimit::max(API_JSON_BODY_LIMIT_BYTES))
        .layer(cors)
        .with_state(state)
}
