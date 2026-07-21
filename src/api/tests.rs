//! API route tests.

use super::*;
use axum::body::{Body, to_bytes};
use axum::http::Request as HttpRequest;
use serde_json::json;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

use crate::conversation::{MessageMetadata, MessagePart, ToolCall};
use crate::mcp::McpCommand;
use crate::session::{SessionId, SessionStatus};
use crate::tool::{ToolAnnotations, ToolPermission, ToolProviderKind, ToolProviderRef};

static TEMP_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn assistant_delta_event_uses_matching_sse_name_and_json_type() {
    let event = crate::session::SessionEvent::AssistantDelta {
        text: "hello".to_string(),
    };
    let body = serde_json::to_value(&event).unwrap();

    assert_eq!(event.event_name(), "assistant_delta");
    assert_eq!(body["type"], "assistant_delta");
    assert_eq!(body["text"], "hello");
}

#[test]
fn reasoning_delta_event_uses_matching_sse_name_and_json_type() {
    let event = crate::session::SessionEvent::ReasoningDelta {
        text: "thinking".to_string(),
    };
    let body = serde_json::to_value(&event).unwrap();

    assert_eq!(event.event_name(), "reasoning_delta");
    assert_eq!(body["type"], "reasoning_delta");
    assert_eq!(body["text"], "thinking");
}

#[test]
fn tool_call_delta_event_uses_matching_sse_name_and_json_type() {
    let event = crate::session::SessionEvent::ToolCallDelta {
        index: 0,
        id: Some("call_123".to_string()),
        name: Some("run_shell".to_string()),
        arguments_delta: Some(r#"{"command""#.to_string()),
    };
    let body = serde_json::to_value(&event).unwrap();

    assert_eq!(event.event_name(), "tool_call_delta");
    assert_eq!(body["type"], "tool_call_delta");
    assert_eq!(body["index"], 0);
    assert_eq!(body["id"], "call_123");
    assert_eq!(body["name"], "run_shell");
    assert_eq!(body["arguments_delta"], r#"{"command""#);
}

#[tokio::test]
async fn health_does_not_require_token() {
    let app = test_app(temp_database_path());
    let response = app
        .oneshot(
            HttpRequest::builder()
                .method(Method::GET)
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["ok"], true);
}

#[tokio::test]
async fn protected_routes_reject_missing_or_invalid_token() {
    let app = test_app(temp_database_path());
    let missing = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method(Method::GET)
                .uri("/api/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let invalid = app
        .oneshot(
            HttpRequest::builder()
                .method(Method::GET)
                .uri("/api/conversations")
                .header(API_TOKEN_HEADER, "wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn typed_windie_errors_map_to_http_status_codes() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let missing = app
        .clone()
        .oneshot(authed_request(
            Method::GET,
            "/api/conversations/missing",
            None,
        ))
        .await
        .unwrap();

    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();
    let invalid = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/messages"),
            Some(json!({"role":"user","text":""})),
        ))
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn conversation_model_route_persists_model() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();

    let updated = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/model"),
                Some(json!({"model":"anthropic/test"})),
            ))
            .await
            .unwrap(),
    )
    .await;
    let inspected = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::GET,
                &format!("/api/conversations/{conversation_id}"),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    let listed = response_json(
        app.oneshot(authed_request(Method::GET, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(updated["model"], "anthropic/test");
    assert_eq!(inspected["model"], "anthropic/test");
    assert_eq!(listed["conversations"][0]["model"], "anthropic/test");

    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn conversation_reasoning_route_persists_reasoning_effort() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();

    let updated = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/reasoning"),
                Some(json!({"effort":"high"})),
            ))
            .await
            .unwrap(),
    )
    .await;
    let inspected = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::GET,
                &format!("/api/conversations/{conversation_id}"),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    let cleared = response_json(
        app.oneshot(authed_request(
            Method::PATCH,
            &format!("/api/conversations/{conversation_id}/reasoning"),
            Some(json!({"effort":null})),
        ))
        .await
        .unwrap(),
    )
    .await;

    assert_eq!(updated["reasoning"]["effort"], "high");
    assert_eq!(inspected["reasoning"]["effort"], "high");
    assert_eq!(cleared["reasoning"], Value::Null);

    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn conversation_image_route_returns_scoped_image_bytes() {
    let db_path = temp_database_path();
    let conversation_id = {
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        store
            .insert_message_with_parts(
                &conversation_id,
                None,
                Role::User,
                "image",
                &[
                    crate::conversation::UnsavedMessagePart::Text("image".to_string()),
                    crate::conversation::UnsavedMessagePart::Image(
                        crate::conversation::UnsavedImagePart {
                            mime_type: "image/png".to_string(),
                            bytes: vec![1, 2, 3],
                        },
                    ),
                ],
                None,
            )
            .unwrap();
        conversation_id
    };
    let asset_id = {
        let store = Store::open_at(&db_path).unwrap();
        let messages = store.load_messages(&conversation_id).unwrap();
        match &messages[0].parts[1] {
            MessagePart::Image(image) => image.asset_id.as_str().to_string(),
            _ => panic!("expected image part"),
        }
    };
    let app = test_app(db_path.clone());

    let response = app
        .clone()
        .oneshot(authed_request(
            Method::GET,
            &format!("/api/conversations/{conversation_id}/images/{asset_id}"),
            None,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], &[1, 2, 3]);

    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn insert_tool_role_message_returns_raw_error() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();

    let response = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/messages"),
            Some(json!({"role":"tool","text":"tool output"})),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "role: tool messages must be created through approve or deny"
    );
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn insert_message_accepts_image_data_part() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();

    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/messages"),
                Some(json!({
                    "role": "user",
                    "parts": [
                        {"type": "text", "text": "clipboard image"},
                        {
                            "type": "image_data",
                            "mime_type": "image/png",
                            "data": "iVBORw0KGgo="
                        }
                    ]
                })),
            ))
            .await
            .unwrap(),
    )
    .await;

    let report = response_json(
        app.oneshot(authed_request(
            Method::GET,
            &format!("/api/conversations/{conversation_id}"),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;
    let parts = report["messages"][0]["parts"].as_array().unwrap();
    assert_eq!(parts[1]["type"], "image");
    assert_eq!(parts[1]["mime_type"], "image/png");
    assert_eq!(parts[1]["byte_count"], 8);

    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn routes_share_operations_for_conversation_inspection_flow() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();

    let listed = response_json(
        app.clone()
            .oneshot(authed_request(Method::GET, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(listed["conversations"].as_array().unwrap().len(), 1);

    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/messages"),
                Some(json!({"role":"user","text":"hello from api"})),
            ))
            .await
            .unwrap(),
    )
    .await;
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/system-prompt"),
                Some(json!({"text":"Use short answers."})),
            ))
            .await
            .unwrap(),
    )
    .await;
    let tool_mode = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/tool-approval-mode"),
                Some(json!({"mode":"auto_approve_attached"})),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(tool_mode["tool_approval_mode"], "auto_approve_attached");
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/tool-schemas"),
                Some(json!({
                    "name":"run_shell",
                    "description":"Run a command",
                    "parameters":{"type":"object"}
                })),
            ))
            .await
            .unwrap(),
    )
    .await;

    let inspected = response_json(
        app.oneshot(authed_request(
            Method::GET,
            &format!("/api/conversations/{conversation_id}?model=openai/test"),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;

    assert_eq!(inspected["system_prompt"], "Use short answers.");
    assert_eq!(inspected["tool_approval_mode"], "auto_approve_attached");
    assert_eq!(inspected["messages"][0]["content"], "hello from api");
    assert!(inspected["path"].as_array().unwrap().is_empty());
    assert_eq!(inspected["model_context"][0]["role"], "system");
    assert_eq!(inspected["tool_schemas"][0]["name"], "run_shell");
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn update_remove_and_schema_routes_share_operations() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();
    let inserted = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/messages"),
                Some(json!({"role":"user","text":"before"})),
            ))
            .await
            .unwrap(),
    )
    .await;
    let message_id = inserted["message_id"].as_str().unwrap();

    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/messages/{message_id}"),
                Some(json!({"text":"after"})),
            ))
            .await
            .unwrap(),
    )
    .await;
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/tool-schemas"),
                Some(json!({
                    "name":"first_tool",
                    "description":"First",
                    "parameters":{"type":"object"}
                })),
            ))
            .await
            .unwrap(),
    )
    .await;
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/tool-schemas/first_tool"),
                Some(json!({
                    "name":"second_tool",
                    "description":"Second",
                    "parameters":{"type":"object","properties":{}}
                })),
            ))
            .await
            .unwrap(),
    )
    .await;
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::DELETE,
                &format!("/api/conversations/{conversation_id}/tool-schemas/second_tool"),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::DELETE,
                &format!("/api/conversations/{conversation_id}/messages/{message_id}"),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    response_json(
        app.clone()
            .oneshot(authed_request(
                Method::DELETE,
                &format!("/api/conversations/{conversation_id}"),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;

    let listed = response_json(
        app.oneshot(authed_request(Method::GET, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    assert!(listed["conversations"].as_array().unwrap().is_empty());
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn system_prompt_and_tools_are_tree_wide() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let mut store = Store::open_at(&db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let branch_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::User,
            "branch",
            None,
        )
        .unwrap();
    let sibling_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::User,
            "sibling",
            None,
        )
        .unwrap();
    drop(store);

    let prompt_response = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/system-prompt"),
                Some(json!({
                    "text": "global prompt"
                })),
            ))
            .await
            .unwrap(),
    )
    .await;
    let tool_response = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/tool-schemas"),
                Some(json!({
                    "name": "global_tool",
                    "description": "Global tool",
                    "parameters": {"type": "object"}
                })),
            ))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(prompt_response["system_prompt"], "global prompt");
    assert_eq!(tool_response["name"], "global_tool");

    let store = Store::open_at(&db_path).unwrap();

    assert_eq!(
        store
            .system_prompt(&conversation_id)
            .unwrap()
            .as_deref(),
        Some("global prompt")
    );
    assert_eq!(
        store.load_tool_schemas(&conversation_id).unwrap()[0]
            .name
            .as_str(),
        "global_tool"
    );
    // Tree-wide: both branches see same
    assert!(store.load_path_to_message(&conversation_id, &branch_id).is_ok());
    assert!(store.load_path_to_message(&conversation_id, &sibling_id).is_ok());
    assert_eq!(store.system_prompt(&conversation_id).unwrap().as_deref(), Some("global prompt"));
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn batch_attach_tools_route_attaches_provider_tools() {
    let db_path = temp_database_path();
    let app =
        test_app_with_tool_registry(db_path.clone(), Arc::new(registry_with_cached_test_tool()));
    let created = response_json(
        app.clone()
            .oneshot(authed_request(Method::POST, "/api/conversations", None))
            .await
            .unwrap(),
    )
    .await;
    let conversation_id = created["conversation_id"].as_str().unwrap();

    let attached = response_json(
        app.clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/tools/batch"),
                Some(json!({
                    "tools": [
                        {
                            "provider_id": "desktop-commander",
                            "tool_name": "read_file"
                        }
                    ]
                })),
            ))
            .await
            .unwrap(),
    )
    .await;
    let inspected = response_json(
        app.oneshot(authed_request(
            Method::GET,
            &format!("/api/conversations/{conversation_id}"),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;

    assert_eq!(attached["names"], json!(["desktop_commander__read_file"]));
    assert_eq!(
        inspected["tool_schemas"][0]["name"],
        "desktop_commander__read_file"
    );
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn remove_tool_result_message_deletes_tool_group() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let mut store = Store::open_at(&db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let parent_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&parent_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function("call_1", "run_shell", "{}")],
                ..Default::default()
            }),
        )
        .unwrap();
    let message_id = store
        .insert_tool_result_message(
            &conversation_id,
            &assistant_id,
            &ToolCallId::new("call_1"),
            "{}",
        )
        .unwrap();
    drop(store);

    let response = app
        .oneshot(authed_request(
            Method::DELETE,
            &format!("/api/conversations/{conversation_id}/messages/{message_id}"),
            None,
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;
    let store = Store::open_at(&db_path).unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deleted"], true);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id.as_ref(), Some(&parent_id));
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn approve_later_multi_tool_call_records_raw_order_error() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let conversation_id = insert_multi_tool_call_assistant(&db_path);
    let head_message_id = latest_message_id(&db_path, &conversation_id);
    let session_id = create_waiting_run(&db_path, &conversation_id, &head_message_id);

    let response = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/sessions/{session_id}/approvals/call_2/approve"),
            None,
        ))
        .await
        .unwrap();
    let status = response.status();
    let _body = response_json_body(response).await;
    let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(session.status, SessionStatus::Failed);
    assert!(
        session
            .error
            .as_deref()
            .unwrap_or("")
            .contains("tool call must be resolved after previous tool call: call_1")
    );
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn deny_first_multi_tool_call_records_result_without_querying() {
    let db_path = temp_database_path();
    let registry = Arc::new(registry_with_cached_test_tool());
    let app = test_app_with_tool_registry(db_path.clone(), registry.clone());
    let conversation_id = insert_attached_multi_tool_call_assistant(&db_path);
    let head_message_id = latest_message_id(&db_path, &conversation_id);
    let session_id = create_waiting_run(&db_path, &conversation_id, &head_message_id);

    let response = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/sessions/{session_id}/approvals/call_1/deny"),
            None,
        ))
        .await
        .unwrap();
    let status = response.status();
    let _body = response_json_body(response).await;
    let events = wait_for_run_event_count(&db_path, &session_id, 2).await;
    let session = Store::open_at(&db_path)
        .unwrap()
        .load_session(&session_id)
        .unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(session.status, SessionStatus::WaitingForApproval);
    assert!(events.iter().any(|event| matches!(
        event.event,
        crate::session::SessionEvent::ToolResultSaved { .. }
    )));
    assert!(events.iter().any(|event| matches!(
        event.event,
        crate::session::SessionEvent::WaitingForApproval
    )));
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn create_session_records_gateway_error() {
    let db_path = temp_database_path();
    let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
    let mut store = Store::open_at(&db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let head_message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    drop(store);

    let response = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/sessions"),
            Some(json!({"head_message_id": head_message_id.as_str(), "model":"openai/test"})),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;
    assert_eq!(body["status"], "ready");
    let session_id = SessionId::new(body["id"].as_str().unwrap());
    let query = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/sessions/{session_id}/query"),
            Some(json!({"text":"hello"})),
        ))
        .await
        .unwrap();
    assert_eq!(query.status(), StatusCode::OK);
    let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(session.status, SessionStatus::Failed);
    assert_ne!(session.current_head_message_id.as_ref(), Some(&head_message_id));
    assert_eq!(
        session.error.as_deref(),
        Some("Bifrost is not running. Start it with: windie gateway start")
    );
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn query_session_advances_branch_from_requested_head() {
    let db_path = temp_database_path();
    let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
    let mut store = Store::open_at(&db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let head_message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    drop(store);

    let response = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/sessions"),
            Some(json!({
                "head_message_id": head_message_id.as_str(),
                "model":"openai/test"
            })),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;
    assert_eq!(body["status"], "ready");
    let session_id = SessionId::new(body["id"].as_str().unwrap());
    let query = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/sessions/{session_id}/query"),
            Some(json!({"text":"hello"})),
        ))
        .await
        .unwrap();
    assert_eq!(query.status(), StatusCode::OK);
    let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        session.start_head_message_id.as_ref(),
        Some(&head_message_id)
    );
    // The query appended a user message under the requested head and advanced
    // the branch head, so the current head moved past the requested head.
    assert_ne!(
        session.current_head_message_id.as_ref(),
        Some(&head_message_id)
    );
    assert_eq!(session.status, SessionStatus::Failed);
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn session_events_replay_gateway_errors_as_sse_events() {
    let db_path = temp_database_path();
    let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
    let mut store = Store::open_at(&db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let head_message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    drop(store);

    let response = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/sessions"),
            Some(json!({"head_message_id": head_message_id.as_str(), "model":"openai/test"})),
        ))
        .await
        .unwrap();
    let body = response_json_body(response).await;
    let session_id = SessionId::new(body["id"].as_str().unwrap());
    let query = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/sessions/{session_id}/query"),
            Some(json!({"text":"hello"})),
        ))
        .await
        .unwrap();
    assert_eq!(query.status(), StatusCode::OK);
    let _run = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

    let response = app
        .oneshot(authed_request(
            Method::GET,
            &format!("/api/sessions/{session_id}/events"),
            None,
        ))
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/event-stream"));
    assert!(body.contains("event: failed"));
    assert!(body.contains("Bifrost is not running. Start it with: windie gateway start"));
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn session_survives_event_stream_client_disconnect() {
    let db_path = temp_database_path();
    let mock = spawn_mock_bifrost(Duration::from_millis(100)).await;
    let app = test_app_with_urls(db_path.clone(), &mock.gateway_url, &mock.base_url);
    let mut store = Store::open_at(&db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let head_message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    drop(store);

    let response = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/sessions"),
            Some(json!({"head_message_id": head_message_id.as_str(), "model":"openai/test"})),
        ))
        .await
        .unwrap();
    let body = response_json(response).await;
    let session_id = SessionId::new(body["id"].as_str().unwrap());
    let query = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/sessions/{session_id}/query"),
            Some(json!({"text":"hello"})),
        ))
        .await
        .unwrap();
    assert_eq!(query.status(), StatusCode::OK);

    let response = app
        .oneshot(authed_request(
            Method::GET,
            &format!("/api/sessions/{session_id}/events"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);

    let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Completed).await;
    let store = Store::open_at(&db_path).unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();
    let events = store.load_session_events_after(&session_id, None).unwrap();

    assert_eq!(session.status, SessionStatus::Completed);
    assert!(session.error.is_none());
    assert!(messages.iter().any(|message| {
        message.role == Role::Assistant && message.content == "after disconnect"
    }));
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, crate::session::SessionEvent::Completed { .. }))
    );
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn model_list_route_returns_gateway_error_when_gateway_is_offline() {
    let db_path = temp_database_path();
    let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");

    let response = app
        .oneshot(authed_request(Method::GET, "/api/models", None))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body["error"],
        "Bifrost is not running. Start it with: windie gateway start"
    );
    let _ = fs::remove_file(db_path);
}

fn test_app(store_path: PathBuf) -> Router {
    test_app_with_gateway(store_path, "http://localhost:8080")
}

fn test_app_with_gateway(store_path: PathBuf, gateway_url: &str) -> Router {
    test_app_with_urls(store_path, gateway_url, "http://localhost:8080/v1")
}

fn test_app_with_urls(store_path: PathBuf, gateway_url: &str, base_url: &str) -> Router {
    let tool_registry = Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions());
    let session_manager = Arc::new(SessionManager::new(
        Some(store_path.clone()),
        gateway_url.to_string(),
        base_url.to_string(),
        tool_registry.clone(),
    ));
    router(ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: base_url.to_string(),
        model: "openai/test".to_string(),
        api_token: "test-token".to_string(),
        store_path: Some(store_path),
        tool_registry,
        session_manager,
    })
}

fn test_app_with_tool_registry(
    store_path: PathBuf,
    tool_registry: Arc<ToolProviderRegistry>,
) -> Router {
    let session_manager = Arc::new(SessionManager::new(
        Some(store_path.clone()),
        "http://localhost:8080".to_string(),
        "http://localhost:8080/v1".to_string(),
        tool_registry.clone(),
    ));
    router(ApiState {
        gateway_url: "http://localhost:8080".to_string(),
        base_url: "http://localhost:8080/v1".to_string(),
        model: "openai/test".to_string(),
        api_token: "test-token".to_string(),
        store_path: Some(store_path),
        tool_registry,
        session_manager,
    })
}

struct MockBifrost {
    gateway_url: String,
    base_url: String,
}

async fn spawn_mock_bifrost(response_delay: Duration) -> MockBifrost {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(handle_mock_bifrost_connection(stream, response_delay));
        }
    });

    MockBifrost {
        gateway_url: format!("http://{address}"),
        base_url: format!("http://{address}/v1"),
    }
}

async fn handle_mock_bifrost_connection(
    mut stream: tokio::net::TcpStream,
    response_delay: Duration,
) {
    let mut buffer = vec![0_u8; 8192];
    let Ok(size) = stream.read(&mut buffer).await else {
        return;
    };
    let request = String::from_utf8_lossy(&buffer[..size]);

    if request.starts_with("GET /health ") {
        write_mock_response(&mut stream, "text/plain", "ok").await;
    } else if request.starts_with("GET /api/models/parameters") {
        write_mock_response(
            &mut stream,
            "application/json",
            r#"{"model_parameters":[],"supports_reasoning":false,"supports_prompt_caching":false}"#,
        )
        .await;
    } else if request.starts_with("POST /v1/responses ") {
        tokio::time::sleep(response_delay).await;
        write_mock_response(
            &mut stream,
            "text/event-stream",
            concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"after disconnect\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                "data: [DONE]\n\n"
            ),
        )
        .await;
    } else {
        write_mock_status(&mut stream, 404, "not found").await;
    }
}

async fn write_mock_response(stream: &mut tokio::net::TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

async fn write_mock_status(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} Error\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

fn authed_request(method: Method, uri: &str, body: Option<Value>) -> HttpRequest<Body> {
    let mut builder = HttpRequest::builder()
        .method(method)
        .uri(uri)
        .header(API_TOKEN_HEADER, "test-token");

    let body = match body {
        Some(value) => {
            builder = builder.header(CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };

    builder.body(body).unwrap()
}

async fn response_json(response: Response) -> Value {
    assert!(
        response.status().is_success(),
        "unexpected response status: {}",
        response.status()
    );
    response_json_body(response).await
}

async fn response_json_body(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn wait_for_session_status(
    db_path: &PathBuf,
    session_id: &SessionId,
    status: SessionStatus,
) -> crate::session::Session {
    for _ in 0..50 {
        let store = Store::open_at(db_path).unwrap();
        let session = store.load_session(session_id).unwrap();
        if session.status == status {
            return session;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    Store::open_at(db_path)
        .unwrap()
        .load_session(session_id)
        .unwrap()
}

fn create_waiting_run(
    db_path: &PathBuf,
    conversation_id: &ConversationId,
    head_message_id: &MessageId,
) -> SessionId {
    let mut store = Store::open_at(db_path).unwrap();
    let session_id = SessionId::fresh();
    store
        .create_session(
            &session_id,
            conversation_id,
            Some(head_message_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_status(&session_id, SessionStatus::WaitingForApproval, None)
        .unwrap();
    session_id
}

fn latest_message_id(db_path: &PathBuf, conversation_id: &ConversationId) -> MessageId {
    Store::open_at(db_path)
        .unwrap()
        .load_message_tree(conversation_id)
        .unwrap()
        .last()
        .and_then(|message| message.id.clone())
        .expect("test fixture should have a latest message")
}

async fn wait_for_run_event_count(
    db_path: &PathBuf,
    session_id: &SessionId,
    count: usize,
) -> Vec<SessionEventRecord> {
    for _ in 0..50 {
        let store = Store::open_at(db_path).unwrap();
        let events = store.load_session_events_after(session_id, None).unwrap();
        if events.len() >= count {
            return events;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    Store::open_at(db_path)
        .unwrap()
        .load_session_events_after(session_id, None)
        .unwrap()
}

fn registry_with_cached_test_tool() -> ToolProviderRegistry {
    ToolProviderRegistry::with_test_mcp_provider(
        "desktop-commander",
        "desktop_commander",
        "Desktop Commander",
        McpCommand {
            program: "windie-test-unused-mcp-provider",
            args: &[],
            env: &[],
        },
        vec![desktop_commander_read_file_definition()],
    )
}

fn desktop_commander_read_file_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: ToolSchemaName::new("desktop_commander__read_file"),
        display_name: "Desktop Commander read_file".to_string(),
        description: "Read a file through Desktop Commander.".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("desktop-commander"),
            ProviderToolName::new("read_file"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

fn insert_multi_tool_call_assistant(db_path: &PathBuf) -> ConversationId {
    let mut store = Store::open_at(db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "run commands", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![
                    ToolCall::function("call_1", "run_shell", r#"{"command":"printf first"}"#),
                    ToolCall::function("call_2", "run_shell", r#"{"command":"printf second"}"#),
                ],
                ..Default::default()
            }),
        )
        .unwrap();

    conversation_id
}

fn insert_attached_multi_tool_call_assistant(db_path: &PathBuf) -> ConversationId {
    let mut store = Store::open_at(db_path).unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let registry = registry_with_cached_test_tool();
    operation::attach_tool_with_registry(
        &mut store,
        &conversation_id,
        &ToolProviderId::new("desktop-commander"),
        &ProviderToolName::new("read_file"),
        &registry,
    )
    .unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "read files", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![
                    ToolCall::function(
                        "call_1",
                        "desktop_commander__read_file",
                        r#"{"path":"/tmp/one"}"#,
                    ),
                    ToolCall::function(
                        "call_2",
                        "desktop_commander__read_file",
                        r#"{"path":"/tmp/two"}"#,
                    ),
                ],
                ..Default::default()
            }),
        )
        .unwrap();

    conversation_id
}

fn temp_database_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEMP_DB_COUNTER.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "windie-api-{}-{nanos}-{counter}.db",
        std::process::id()
    ))
}
