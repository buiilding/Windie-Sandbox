//! Local API route and response tests.

use super::*;
use axum::body::{Body, to_bytes};
use axum::http::Request as HttpRequest;
use serde_json::json;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

use crate::mcp::McpCommand;
use crate::tool::{ToolAnnotations, ToolPermission, ToolProviderKind, ToolProviderRef};

static TEMP_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    assert_eq!(inspected["active_path"][0]["content"], "hello from api");
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
async fn approve_later_multi_tool_call_returns_order_error() {
    let db_path = temp_database_path();
    let app = test_app(db_path.clone());
    let conversation_id = insert_multi_tool_call_assistant(&db_path);

    let response = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/approvals/call_2/approve"),
            None,
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "tool call must be resolved after previous tool call: call_1"
    );
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn deny_first_multi_tool_call_returns_result_without_querying() {
    let db_path = temp_database_path();
    let registry = Arc::new(registry_with_cached_test_tool());
    let app = test_app_with_tool_registry(db_path.clone(), registry.clone());
    let conversation_id = insert_attached_multi_tool_call_assistant(&db_path);

    let response = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/approvals/call_1/deny"),
            None,
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json_body(response).await;
    let store = Store::open_at(&db_path).unwrap();
    let approvals =
        operation::list_tool_approvals_with_registry(&store, &conversation_id, &registry).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tool_call_id"], "call_1");
    assert_eq!(body["tool_name"], "desktop_commander__read_file");
    assert_eq!(body["success"], false);
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_2");
    let _ = fs::remove_file(db_path);
}

#[tokio::test]
async fn query_with_unresolved_tool_call_returns_gateway_error_before_runtime_query() {
    let db_path = temp_database_path();
    let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
    let conversation_id = insert_multi_tool_call_assistant(&db_path);

    let response = app
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/query"),
            Some(json!({"model":"openai/test"})),
        ))
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

#[tokio::test]
async fn durable_run_persists_terminal_events_for_replay() {
    let db_path = temp_database_path();
    let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
    let created = app
        .clone()
        .oneshot(authed_request(Method::POST, "/api/conversations", None))
        .await
        .unwrap();
    let conversation_id = response_json(created).await["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();
    let started = app
        .clone()
        .oneshot(authed_request(
            Method::POST,
            &format!("/api/conversations/{conversation_id}/runs"),
            Some(json!({"model": null, "reasoning": null})),
        ))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let run_id = response_json(started).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    for _ in 0..50 {
        let response = app
            .clone()
            .oneshot(authed_request(
                Method::GET,
                &format!("/api/runs/{run_id}"),
                None,
            ))
            .await
            .unwrap();
        let status = response_json(response).await["status"]
            .as_str()
            .unwrap()
            .to_string();
        if status == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let replay = app
        .oneshot(authed_request(
            Method::GET,
            &format!("/api/runs/{run_id}/events?after=0"),
            None,
        ))
        .await
        .unwrap();
    let body = to_bytes(replay.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("event: query_error"));
    assert!(body.contains(&format!(r#""run_id":"{run_id}""#)));
    assert!(body.contains("\"sequence\":1"));

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
    let run_manager = Arc::new(RunManager::new(Some(store_path.clone())).unwrap());
    router(ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: "http://localhost:8080/v1".to_string(),
        model: "openai/test".to_string(),
        api_token: "test-token".to_string(),
        store_path: Some(store_path),
        tool_registry: Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions()),
        run_manager,
    })
}

fn test_app_with_tool_registry(
    store_path: PathBuf,
    tool_registry: Arc<ToolProviderRegistry>,
) -> Router {
    let run_manager = Arc::new(RunManager::new(Some(store_path.clone())).unwrap());
    router(ApiState {
        gateway_url: "http://localhost:8080".to_string(),
        base_url: "http://localhost:8080/v1".to_string(),
        model: "openai/test".to_string(),
        api_token: "test-token".to_string(),
        store_path: Some(store_path),
        tool_registry,
        run_manager,
    })
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
