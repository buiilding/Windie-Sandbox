//! Named benchmark scenarios for the current Windie architecture.
//!
//! Each scenario declares its architectural layer and fixture so storage,
//! context, runtime, session, serialization, MCP, and lifecycle work can be
//! compared without conflating unrelated operations.

use super::*;
use crate::session::{SessionEvent, SessionId, SessionStatus};
use crate::tool::{ToolSchema, ToolSchemaName};

/// Runs the deterministic provider-free benchmark scenarios selected by the
/// caller. External HTTP, provider installation, and process lifecycle work
/// intentionally stays out of this default path.
pub(super) async fn run(categories: &[BenchmarkCategory]) -> Result<Vec<ScenarioTiming>> {
    let mut scenarios = Vec::new();

    if categories.contains(&BenchmarkCategory::Persistence) {
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "open_database",
            "fresh file-backed SQLite database",
            || {
                let path = runtime_database_path("scenario-store-open");
                let started = Instant::now();
                let _ = Store::open_at(&path)?;
                let duration = started.elapsed();
                remove_runtime_database_files(&path);
                Ok(duration)
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "conversation_list_load",
            "1 conversation, 0 messages",
            || {
                with_runtime_store("scenario-conversation-list", |store| {
                    let _ = store.create_conversation("openai/test")?;
                    let started = Instant::now();
                    let _ = store.list_conversations()?;
                    Ok(started.elapsed())
                })
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "tree_rows_load",
            "linear conversation, 100 total messages",
            || {
                with_runtime_store("scenario-tree-rows", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    let _ = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
                    let started = Instant::now();
                    let _ = store.load_message_rows(&conversation_id)?;
                    Ok(started.elapsed())
                })
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "selected_path_rows_load",
            "linear conversation, selected path contains 100 messages",
            || benchmark_path_rows_load(SCALE_PATH_MESSAGES),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "tree_messages_load",
            "linear conversation, 100 messages including parts",
            || {
                with_runtime_store("scenario-tree-messages", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    let _ = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
                    let started = Instant::now();
                    let _ = store.load_messages(&conversation_id)?;
                    Ok(started.elapsed())
                })
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "selected_path_messages_load",
            "linear conversation, selected path contains 100 messages including parts",
            || benchmark_path_load(SCALE_PATH_MESSAGES),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "branched_tree_rows_load",
            "branched tree, 1,000 total messages",
            || benchmark_branched_rows_load(false),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "branched_selected_path_rows_load",
            "branched tree, 100-message selected path",
            || benchmark_branched_rows_load(true),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "image_message_load",
            "1 message with text plus one PNG image",
            || {
                with_runtime_store("scenario-message-parts", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    store.insert_message_with_parts(
                        &conversation_id,
                        None,
                        Role::User,
                        "image",
                        &[
                            UnsavedMessagePart::Text("image".to_string()),
                            UnsavedMessagePart::Image(UnsavedImagePart {
                                mime_type: "image/png".to_string(),
                                bytes: tiny_png_bytes().to_vec(),
                            }),
                        ],
                        None,
                    )?;
                    let started = Instant::now();
                    let _ = store.load_messages(&conversation_id)?;
                    Ok(started.elapsed())
                })
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "system_prompt_load",
            "1 conversation-wide prompt",
            || {
                with_runtime_store("scenario-system-prompt", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    store.set_system_prompt(
                        &conversation_id,
                        "You are a careful local assistant.",
                    )?;
                    let started = Instant::now();
                    let _ = store.system_prompt(&conversation_id)?;
                    Ok(started.elapsed())
                })
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "compaction_load",
            "1 checkpoint after a 100-message path",
            || {
                with_runtime_store("scenario-compaction", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    let head = create_message_chain(store, &conversation_id, 100)?
                        .ok_or_else(|| anyhow::anyhow!("compaction fixture has no head"))?;
                    store.save_compaction(&conversation_id, &head, "previous history")?;
                    let started = Instant::now();
                    let _ = store.latest_compaction(&conversation_id)?;
                    Ok(started.elapsed())
                })
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Persistence,
            "storage",
            "tool_schema_load",
            "1 attached tool schema",
            || {
                with_runtime_store("scenario-tool-schema", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    store.insert_tool_schema(&conversation_id, &manual_schema("read_file"))?;
                    let started = Instant::now();
                    let _ = store.load_tool_schemas(&conversation_id)?;
                    Ok(started.elapsed())
                })
            },
        )?;
    }

    if categories.contains(&BenchmarkCategory::Conversation) {
        push(
            &mut scenarios,
            BenchmarkCategory::Conversation,
            "context",
            "build_plain_context",
            "100-message selected path, no prompt or compaction",
            || benchmark_model_context(false, false, false),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Conversation,
            "context",
            "build_context_with_system_prompt",
            "100-message selected path plus 1 system prompt",
            || benchmark_model_context(true, false, false),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Conversation,
            "context",
            "build_context_with_compaction",
            "100-message selected path plus 1 compaction",
            || benchmark_model_context(false, true, false),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Conversation,
            "context",
            "build_image_context",
            "1 message with text plus one PNG image",
            || benchmark_model_context(false, false, true),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Conversation,
            "context",
            "build_tool_chain_context",
            "4-message path with 2 completed tool results",
            || {
                with_runtime_store("scenario-context-tool-chain", |store| {
                    let conversation_id = store.create_conversation("openai/test")?;
                    attach_test_mcp_tool(store, &conversation_id)?;
                    let assistant = create_completed_tool_chain(store, &conversation_id, 2)?;
                    let started = Instant::now();
                    let context = ContextBuilder::build_model_context(
                        store,
                        &conversation_id,
                        Some(&assistant),
                    )?;
                    debug_assert!(!context.messages.is_empty());
                    Ok(started.elapsed())
                })
            },
        )?;
    }

    if categories.contains(&BenchmarkCategory::Serialization) {
        push(
            &mut scenarios,
            BenchmarkCategory::Serialization,
            "serialization",
            "responses_text",
            "100-message text context",
            || benchmark_serialization(false, false),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Serialization,
            "serialization",
            "responses_image",
            "1-message text/image context",
            || benchmark_serialization(true, false),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Serialization,
            "serialization",
            "responses_tool_calls",
            "4-message context with 2 tool calls and results",
            || benchmark_serialization(false, true),
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Serialization,
            "serialization",
            "responses_tool_schemas",
            "100-message context plus 1 tool schema",
            benchmark_serialization_with_schema,
        )?;
    }

    if categories.contains(&BenchmarkCategory::Runtime) {
        push_existing(
            &mut scenarios,
            BenchmarkCategory::Runtime,
            "runtime",
            "prepare_plain_completed_turn",
            "100-message path, no tool calls",
            benchmark_prepare_run_head_no_tools,
        )?;
        push_existing(
            &mut scenarios,
            BenchmarkCategory::Runtime,
            "runtime",
            "prepare_completed_tool_chain",
            "12-message path, 10 completed tool results",
            benchmark_prepare_run_head_completed_tool_chain,
        )?;
        push_existing(
            &mut scenarios,
            BenchmarkCategory::Runtime,
            "runtime",
            "prepare_approval_required",
            "2-message path, 1 attached pending tool call",
            benchmark_prepare_run_head_requires_approval,
        )?;
        push_existing(
            &mut scenarios,
            BenchmarkCategory::Runtime,
            "runtime",
            "prepare_policy_denied",
            "2-message path, 1 detached denied tool call",
            benchmark_prepare_run_head_policy_denied,
        )?;
    }

    if categories.contains(&BenchmarkCategory::Sessions) {
        push(
            &mut scenarios,
            BenchmarkCategory::Sessions,
            "sessions",
            "create_and_resolve",
            "1 session, 1 selected head",
            benchmark_session_create_and_resolve,
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Sessions,
            "sessions",
            "queue_and_materialize",
            "2 FIFO inputs, 2 materializations",
            benchmark_session_queue_and_materialize,
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Sessions,
            "sessions",
            "event_append_and_replay",
            "3 durable events, then replay from event 0",
            benchmark_session_event_replay,
        )?;
    }

    if categories.contains(&BenchmarkCategory::Mutations) {
        push(
            &mut scenarios,
            BenchmarkCategory::Mutations,
            "mutations",
            "edit_message",
            "1 text message",
            benchmark_edit_message,
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Mutations,
            "mutations",
            "fork_conversation",
            "copy 100-message selected path into a new conversation",
            benchmark_fork_conversation,
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Mutations,
            "mutations",
            "delete_conversation",
            "delete 100-message conversation plus owned cleanup",
            benchmark_delete_conversation,
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Mutations,
            "mutations",
            "delete_session",
            "terminal session with exclusive 10-message path",
            benchmark_delete_session,
        )?;
    }

    if categories.contains(&BenchmarkCategory::Mcp) {
        push_existing(
            &mut scenarios,
            BenchmarkCategory::Mcp,
            "mcp",
            "protocol_initialize_list_call",
            "fake child process: initialize, list, and call",
            benchmark_fake_mcp_list_call,
        )?;
        let duration = benchmark_registry_mcp_call().await?;
        scenarios.push(ScenarioTiming {
            name: "registry_executor_call".to_string(),
            category: BenchmarkCategory::Mcp,
            layer: "mcp".to_string(),
            fixture: "registry → provider adapter → fake MCP executor".to_string(),
            duration,
        });
    }

    if categories.contains(&BenchmarkCategory::Lifecycle) {
        push(
            &mut scenarios,
            BenchmarkCategory::Lifecycle,
            "lifecycle",
            "uninstall_plan",
            "read-only validation of exact Windie-owned paths",
            || {
                let started = Instant::now();
                let plan = crate::local::uninstall_plan()?;
                debug_assert!(!plan.binaries.is_empty());
                Ok(started.elapsed())
            },
        )?;
        push(
            &mut scenarios,
            BenchmarkCategory::Lifecycle,
            "lifecycle",
            "provider_state_persistence",
            "4 provider state records: install, enable, disable, uninstall",
            benchmark_provider_state_lifecycle,
        )?;
    }

    if categories.contains(&BenchmarkCategory::Api) {
        run_api_scenarios(&mut scenarios).await?;
    }

    Ok(scenarios)
}

async fn run_api_scenarios(scenarios: &mut Vec<ScenarioTiming>) -> Result<()> {
    push_api(
        scenarios,
        "health_route",
        "in-process GET /api/health; empty health response",
        benchmark_api_health().await?,
    );
    push_api(
        scenarios,
        "conversation_list_route",
        "in-process GET /api/conversations; empty database",
        benchmark_api_conversation_list().await?,
    );
    push_api(
        scenarios,
        "conversation_create_route",
        "in-process POST /api/conversations; one SQLite insert",
        benchmark_api_conversation_create().await?,
    );
    push_api(
        scenarios,
        "session_events_sse_route",
        "in-process GET events; one replay event",
        benchmark_api_session_events().await?,
    );
    Ok(())
}

fn push_api(scenarios: &mut Vec<ScenarioTiming>, name: &str, fixture: &str, duration: Duration) {
    scenarios.push(ScenarioTiming {
        name: name.to_string(),
        category: BenchmarkCategory::Api,
        layer: "api".to_string(),
        fixture: fixture.to_string(),
        duration,
    });
}

async fn benchmark_api_health() -> Result<Duration> {
    let path = runtime_database_path("scenario-api-health");
    let _ = Store::open_at(&path)?;
    let app = crate::api::benchmark_router(path.clone());
    let started = Instant::now();
    let response = tower::ServiceExt::oneshot(
        app,
        axum::http::Request::builder()
            .method("GET")
            .uri("/api/health")
            .body(axum::body::Body::empty())?,
    )
    .await?;
    let duration = started.elapsed();
    debug_assert!(response.status().is_success());
    remove_runtime_database_files(&path);
    Ok(duration)
}

async fn benchmark_api_conversation_list() -> Result<Duration> {
    let path = runtime_database_path("scenario-api-conversation-list");
    let _ = Store::open_at(&path)?;
    let app = crate::api::benchmark_router(path.clone());
    let started = Instant::now();
    let response = tower::ServiceExt::oneshot(
        app,
        axum::http::Request::builder()
            .method("GET")
            .uri("/api/conversations")
            .body(axum::body::Body::empty())?,
    )
    .await?;
    let duration = started.elapsed();
    debug_assert!(response.status().is_success());
    remove_runtime_database_files(&path);
    Ok(duration)
}

async fn benchmark_api_conversation_create() -> Result<Duration> {
    let path = runtime_database_path("scenario-api-conversation-create");
    let _ = Store::open_at(&path)?;
    let app = crate::api::benchmark_router(path.clone());
    let started = Instant::now();
    let response = tower::ServiceExt::oneshot(
        app,
        axum::http::Request::builder()
            .method("POST")
            .uri("/api/conversations")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"model":"openai/test"}"#))?,
    )
    .await?;
    let duration = started.elapsed();
    debug_assert!(response.status().is_success());
    remove_runtime_database_files(&path);
    Ok(duration)
}

async fn benchmark_api_session_events() -> Result<Duration> {
    let path = runtime_database_path("scenario-api-session-events");
    let (session_id, conversation_id) = {
        let mut store = Store::open_at(&path)?;
        let conversation_id = store.create_conversation("openai/test")?;
        let session_id = SessionId::fresh();
        store.create_session(&session_id, &conversation_id, None, "openai/test", None)?;
        store.append_session_event(&session_id, SessionEvent::WaitingForApproval)?;
        (session_id, conversation_id)
    };
    let _ = conversation_id;
    let app = crate::api::benchmark_router(path.clone());
    let started = Instant::now();
    let response = tower::ServiceExt::oneshot(
        app,
        axum::http::Request::builder()
            .method("GET")
            .uri(format!(
                "/api/sessions/{}/events?after=0",
                session_id.as_str()
            ))
            .body(axum::body::Body::empty())?,
    )
    .await?;
    let duration = started.elapsed();
    debug_assert!(response.status().is_success());
    remove_runtime_database_files(&path);
    Ok(duration)
}

async fn benchmark_registry_mcp_call() -> Result<Duration> {
    let provider_id = ToolProviderId::new("benchmark-mcp");
    let schema_name = ToolSchemaName::new("benchmark__click");
    let tool = ToolDefinition {
        schema_name: schema_name.clone(),
        display_name: "Benchmark click".to_string(),
        description: "Deterministic benchmark tool.".to_string(),
        parameters: serde_json::json!({"type": "object"}),
        provider: ToolProviderRef::new(
            provider_id.clone(),
            ProviderToolName::new("click"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    };
    let registry = ToolProviderRegistry::with_benchmark_mcp_provider(
        "benchmark-mcp",
        "benchmark",
        "Benchmark",
        FAKE_MCP_COMMAND,
    );
    let attached = tool.attached_tool();
    let call = ToolCall::function("benchmark-call", schema_name.as_str(), "{}");
    let started = Instant::now();
    let result = registry.call_tool(&attached, &call).await?;
    debug_assert!(result.success);
    Ok(started.elapsed())
}

fn push<F>(
    scenarios: &mut Vec<ScenarioTiming>,
    category: BenchmarkCategory,
    layer: &str,
    name: &str,
    fixture: &str,
    measure: F,
) -> Result<()>
where
    F: FnOnce() -> Result<Duration>,
{
    scenarios.push(ScenarioTiming {
        name: name.to_string(),
        category,
        layer: layer.to_string(),
        fixture: fixture.to_string(),
        duration: measure()?,
    });
    Ok(())
}

fn push_existing<F>(
    scenarios: &mut Vec<ScenarioTiming>,
    category: BenchmarkCategory,
    layer: &str,
    name: &str,
    fixture: &str,
    measure: F,
) -> Result<()>
where
    F: FnOnce() -> Result<Duration>,
{
    push(scenarios, category, layer, name, fixture, measure)
}

fn benchmark_model_context(system_prompt: bool, compaction: bool, image: bool) -> Result<Duration> {
    with_runtime_store("scenario-model-context", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = if image {
            store.insert_message_with_parts(
                &conversation_id,
                None,
                Role::User,
                "describe this",
                &[
                    UnsavedMessagePart::Text("describe this".to_string()),
                    UnsavedMessagePart::Image(UnsavedImagePart {
                        mime_type: "image/png".to_string(),
                        bytes: tiny_png_bytes().to_vec(),
                    }),
                ],
                None,
            )?
        } else {
            create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?
                .ok_or_else(|| anyhow::anyhow!("context fixture has no head"))?
        };
        if system_prompt {
            store.set_system_prompt(&conversation_id, "You are a careful assistant.")?;
        }
        if compaction {
            store.save_compaction(&conversation_id, &head, "previous history")?;
        }
        let started = Instant::now();
        let context = ContextBuilder::build_model_context(store, &conversation_id, Some(&head))?;
        debug_assert!(!context.messages.is_empty());
        Ok(started.elapsed())
    })
}

fn benchmark_serialization(image: bool, tool_calls: bool) -> Result<Duration> {
    with_runtime_store("scenario-serialization", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = if tool_calls {
            create_completed_tool_chain(store, &conversation_id, 2)?
        } else if image {
            store.insert_message_with_parts(
                &conversation_id,
                None,
                Role::User,
                "image",
                &[
                    UnsavedMessagePart::Text("image".to_string()),
                    UnsavedMessagePart::Image(UnsavedImagePart {
                        mime_type: "image/png".to_string(),
                        bytes: tiny_png_bytes().to_vec(),
                    }),
                ],
                None,
            )?
        } else {
            create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?
                .ok_or_else(|| anyhow::anyhow!("serialization fixture has no head"))?
        };
        let context = ContextBuilder::build_model_context(store, &conversation_id, Some(&head))?;
        let started = Instant::now();
        let bytes = crate::llm::benchmark_responses_request_size(
            "openai/test",
            &context.messages,
            &context.tool_schemas,
        )?;
        debug_assert!(bytes > 0);
        Ok(started.elapsed())
    })
}

fn benchmark_serialization_with_schema() -> Result<Duration> {
    with_runtime_store("scenario-serialization-schema", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?
            .ok_or_else(|| anyhow::anyhow!("serialization fixture has no head"))?;
        store.insert_tool_schema(&conversation_id, &manual_schema("read_file"))?;
        let context = ContextBuilder::build_model_context(store, &conversation_id, Some(&head))?;
        let started = Instant::now();
        let bytes = crate::llm::benchmark_responses_request_size(
            "openai/test",
            &context.messages,
            &context.tool_schemas,
        )?;
        debug_assert!(bytes > 0);
        Ok(started.elapsed())
    })
}

fn benchmark_session_create_and_resolve() -> Result<Duration> {
    with_runtime_store("scenario-session-resolution", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = insert_user_message(store, &conversation_id, None, "hello")?;
        let session_id = SessionId::fresh();
        let started = Instant::now();
        store.create_session(
            &session_id,
            &conversation_id,
            Some(&head),
            "openai/test",
            None,
        )?;
        let resolution = store.resolve_session_at_head(&conversation_id, Some(&head))?;
        debug_assert!(matches!(
            resolution,
            crate::session::SessionResolution::Existing(_)
        ));
        Ok(started.elapsed())
    })
}

fn benchmark_session_queue_and_materialize() -> Result<Duration> {
    with_runtime_store("scenario-session-queue", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let session_id = SessionId::fresh();
        store.create_session(&session_id, &conversation_id, None, "openai/test", None)?;
        store.enqueue_session_input(&session_id, "first", &[])?;
        store.enqueue_session_input(&session_id, "second", &[])?;
        let started = Instant::now();
        let _ = store.materialize_next_session_input(&session_id)?;
        let _ = store.materialize_next_session_input(&session_id)?;
        Ok(started.elapsed())
    })
}

fn benchmark_session_event_replay() -> Result<Duration> {
    with_runtime_store("scenario-session-events", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let session_id = SessionId::fresh();
        store.create_session(&session_id, &conversation_id, None, "openai/test", None)?;
        let started = Instant::now();
        store.append_session_event(&session_id, SessionEvent::WaitingForApproval)?;
        store.append_session_event(
            &session_id,
            SessionEvent::AssistantDelta {
                text: "hello".to_string(),
            },
        )?;
        store.append_session_event(&session_id, SessionEvent::Completed { message_id: None })?;
        let events = store.load_session_events_after(&session_id, Some(0))?;
        debug_assert_eq!(events.len(), 3);
        Ok(started.elapsed())
    })
}

fn benchmark_edit_message() -> Result<Duration> {
    with_runtime_store("scenario-message-edit", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let message_id = insert_user_message(store, &conversation_id, None, "before")?;
        let started = Instant::now();
        store.replace_message(&conversation_id, &message_id, "after")?;
        Ok(started.elapsed())
    })
}

fn benchmark_fork_conversation() -> Result<Duration> {
    with_runtime_store("scenario-conversation-fork", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?
            .ok_or_else(|| anyhow::anyhow!("fork fixture has no head"))?;
        let started = Instant::now();
        let forked = store.fork_conversation_at_message(&conversation_id, &head)?;
        debug_assert_ne!(forked, conversation_id);
        Ok(started.elapsed())
    })
}

fn benchmark_delete_conversation() -> Result<Duration> {
    with_runtime_store("scenario-conversation-delete", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let _ = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
        let started = Instant::now();
        store.remove_conversation(&conversation_id)?;
        Ok(started.elapsed())
    })
}

fn benchmark_delete_session() -> Result<Duration> {
    with_runtime_store("scenario-session-delete", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = create_message_chain(store, &conversation_id, 10)?
            .ok_or_else(|| anyhow::anyhow!("session fixture has no head"))?;
        let session_id = SessionId::fresh();
        store.create_session(&session_id, &conversation_id, None, "openai/test", None)?;
        store.update_session_head(&session_id, Some(&head))?;
        store.update_session_status(&session_id, SessionStatus::Completed, None)?;
        let started = Instant::now();
        store.remove_session(&session_id)?;
        Ok(started.elapsed())
    })
}

fn benchmark_provider_state_lifecycle() -> Result<Duration> {
    with_runtime_store("scenario-provider-state", |store| {
        let provider_id = ToolProviderId::new("benchmark-provider");
        let started = Instant::now();
        store.install_provider(&provider_id)?;
        store.set_provider_state(
            &provider_id,
            crate::tool::ProviderInstallState::Enabled,
            None,
        )?;
        store.set_provider_state(
            &provider_id,
            crate::tool::ProviderInstallState::Disabled,
            None,
        )?;
        store.uninstall_provider(&provider_id)?;
        Ok(started.elapsed())
    })
}

fn manual_schema(name: &str) -> ToolSchema {
    ToolSchema {
        name: ToolSchemaName::new(name),
        description: "Read a local file.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        }),
    }
}
