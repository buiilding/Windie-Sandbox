//! Provider-free runtime benchmark scenarios.

use super::{
    Arc, BRANCH_CHILDREN, ContextBuilder, Duration, FAKE_MCP_COMMAND, IMAGE_PART_MESSAGES, Instant,
    LARGE_SCALE_PATH_MESSAGES, LARGE_TRUNCATE_DESCENDANTS, Result, Role, RunEvent, RunManager,
    RuntimeBenchmarkTimings, RuntimeContextBenchmark, RuntimeRunAction, SCALE_PATH_MESSAGES, Store,
    TEST_PROVIDER_ID, TOOL_CHAIN_RESULTS, ToolProviderId, ToolProviderKind, ToolProviderRegistry,
    UnsavedImagePart, UnsavedMessagePart, Uuid, attach_test_mcp_tool, create_completed_tool_chain,
    create_message_chain, deny_tool_call, env, fs, insert_tool_result, insert_user_message, mcp,
    operation, pending_tool_approvals, prepare_query_turn, process, remove_runtime_database_files,
    runtime_database_path, test_tool_definition, tiny_png_bytes, tool_call, tool_call_metadata,
    with_runtime_store,
};

/// Runs all provider-free scenarios while keeping fixture construction outside
/// each measured interval.
pub(super) async fn run_runtime_benchmark() -> Result<RuntimeBenchmarkTimings> {
    let prepare_query_turn = benchmark_prepare_query_turn()?;
    let pending_tool_approval_scan = benchmark_pending_tool_approval_scan()?;
    let tool_result_insert = benchmark_tool_result_insert()?;
    let deny_tool_result_persist = benchmark_deny_tool_result_persist()?;
    let (splice_remove, deleted_messages, promoted_children) = benchmark_splice_remove()?;
    let (truncate, truncated_messages) = benchmark_truncate()?;
    let context = benchmark_context_after_tool_chain()?;
    let active_path_load_100 = benchmark_active_path_load(SCALE_PATH_MESSAGES)?;
    let active_path_load_1000 = benchmark_active_path_load(LARGE_SCALE_PATH_MESSAGES)?;
    let pending_tool_approval_scan_long_path = benchmark_pending_tool_approval_scan_long_path()?;
    let pending_tool_approval_scan_deep_chain = benchmark_pending_tool_approval_scan_deep_chain()?;
    let prepare_query_no_tools = benchmark_prepare_query_no_tools()?;
    let prepare_query_completed_tool_chain = benchmark_prepare_query_completed_tool_chain()?;
    let prepare_query_requires_approval = benchmark_prepare_query_requires_approval()?;
    let prepare_query_policy_denied = benchmark_prepare_query_policy_denied()?;
    let (splice_remove_branch_point, _branch_deleted_messages, _branch_promoted_children) =
        benchmark_splice_remove_branch_point()?;
    let (splice_remove_root_many_children, _root_deleted_messages, _root_promoted_children) =
        benchmark_splice_remove_root_many_children()?;
    let splice_remove_tool_group = benchmark_splice_remove_tool_group()?;
    let (truncate_large_subtree, _large_truncated_messages) = benchmark_truncate_large_subtree()?;
    let context_build_plain_100 = benchmark_context_plain(SCALE_PATH_MESSAGES)?;
    let context_build_plain_1000 = benchmark_context_plain(LARGE_SCALE_PATH_MESSAGES)?;
    let context_build_with_system_prompt = benchmark_context_with_system_prompt()?;
    let context_build_with_compaction = benchmark_context_with_compaction()?;
    let context_build_with_image_parts = benchmark_context_with_image_parts()?;
    let provider_tool_attach_load = benchmark_provider_tool_attach_load()?;
    let fake_mcp_list_call = benchmark_fake_mcp_list_call()?;
    let durable_stream_journal = benchmark_durable_stream_journal().await?;
    let inspection_snapshot_1000 = benchmark_inspection_snapshot_1000()?;
    let fork_conversation_1000 = benchmark_fork_conversation_1000()?;
    let run_action_lifecycle = benchmark_run_action_lifecycle().await?;
    let run_admission_contention = benchmark_run_admission_contention().await?;
    let (fake_mcp_catalog_singleflight, provider_catalog_starts) =
        benchmark_fake_mcp_catalog_singleflight()?;

    Ok(RuntimeBenchmarkTimings {
        prepare_query_turn,
        pending_tool_approval_scan,
        tool_result_insert,
        deny_tool_result_persist,
        splice_remove,
        truncate,
        context_build_after_tool_chain: context.duration,
        active_path_load_100,
        active_path_load_1000,
        pending_tool_approval_scan_long_path,
        pending_tool_approval_scan_deep_chain,
        prepare_query_no_tools,
        prepare_query_completed_tool_chain,
        prepare_query_requires_approval,
        prepare_query_policy_denied,
        splice_remove_branch_point,
        splice_remove_root_many_children,
        splice_remove_tool_group,
        truncate_large_subtree,
        context_build_plain_100,
        context_build_plain_1000,
        context_build_with_system_prompt,
        context_build_with_compaction,
        context_build_with_image_parts,
        provider_tool_attach_load,
        fake_mcp_list_call,
        durable_stream_journal,
        inspection_snapshot_1000,
        fork_conversation_1000,
        run_action_lifecycle,
        run_admission_contention,
        fake_mcp_catalog_singleflight,
        active_path_messages: context.active_path_messages,
        tree_messages: context.tree_messages,
        requested_tool_calls: context.requested_tool_calls,
        resolved_tool_results: context.resolved_tool_results,
        deleted_messages,
        promoted_children,
        truncated_messages,
        provider_catalog_starts,
    })
}

/// Measures query preparation on a minimal ready active path.
fn benchmark_prepare_query_turn() -> Result<Duration> {
    with_runtime_store("prepare-query-turn", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        insert_user_message(store, &conversation_id, None, "ready")?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures active-path scanning for the next approval-required tool call.
fn benchmark_pending_tool_approval_scan() -> Result<Duration> {
    with_runtime_store("pending-tool-approval-scan", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let user_id = insert_user_message(store, &conversation_id, None, "use tools")?;
        let metadata = tool_call_metadata(vec![
            tool_call(0, "call_1", "desktop_commander__read_file"),
            tool_call(1, "call_2", "desktop_commander__read_file"),
        ]);
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        let approvals = pending_tool_approvals(store, &conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(approvals.len(), 1);

        Ok(duration)
    })
}

/// Measures raw tool-result message persistence without shell execution.
fn benchmark_tool_result_insert() -> Result<Duration> {
    with_runtime_store("tool-result-insert", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let call = tool_call(0, "call_1", "desktop_commander__read_file");
        let assistant_id = store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(vec![call.clone()])),
        )?;
        let started = Instant::now();
        store.insert_tool_result_message(
            &conversation_id,
            &assistant_id,
            &call.id,
            r#"{"stdout":"ok","stderr":"","exit_code":0}"#,
        )?;
        Ok(started.elapsed())
    })
}

/// Measures explicit denial lookup plus result persistence without shell work.
fn benchmark_deny_tool_result_persist() -> Result<Duration> {
    with_runtime_store("deny-tool-result-persist", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let call = tool_call(0, "call_1", "desktop_commander__read_file");
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(vec![call.clone()])),
        )?;

        let started = Instant::now();
        deny_tool_call(store, &conversation_id, &call.id)?;
        Ok(started.elapsed())
    })
}

/// Measures splice delete for a middle message in a chain.
fn benchmark_splice_remove() -> Result<(Duration, usize, usize)> {
    with_runtime_store("splice-remove", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let first_id = insert_user_message(store, &conversation_id, None, "first")?;
        let removed_id = store.insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "middle",
            None,
        )?;
        let _child_id = insert_user_message(store, &conversation_id, Some(&removed_id), "child")?;

        let started = Instant::now();
        store.remove_message(&conversation_id, &removed_id)?;
        Ok((started.elapsed(), 1, 1))
    })
}

/// Measures descendant deletion below a checkpoint message.
fn benchmark_truncate() -> Result<(Duration, usize)> {
    with_runtime_store("truncate", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let first_id = insert_user_message(store, &conversation_id, None, "first")?;
        let checkpoint_id = store.insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "keep",
            None,
        )?;
        let child_id = insert_user_message(store, &conversation_id, Some(&checkpoint_id), "child")?;
        let _grandchild_id = store.insert_message(
            &conversation_id,
            Some(&child_id),
            Role::Assistant,
            "leaf",
            None,
        )?;

        let started = Instant::now();
        store.truncate_after_message(&conversation_id, &checkpoint_id)?;
        Ok((started.elapsed(), 2))
    })
}

/// Measures context construction after a completed two-result tool chain.
fn benchmark_context_after_tool_chain() -> Result<RuntimeContextBenchmark> {
    with_runtime_store("context-after-tool-chain", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let user_id = insert_user_message(store, &conversation_id, None, "use tools")?;
        let first_call = tool_call(0, "call_1", "desktop_commander__read_file");
        let second_call = tool_call(1, "call_2", "desktop_commander__read_file");
        let assistant_id = store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(vec![
                first_call.clone(),
                second_call.clone(),
            ])),
        )?;
        let first_result_id =
            insert_tool_result(store, &conversation_id, &assistant_id, &first_call.id)?;
        let _second_result_id =
            insert_tool_result(store, &conversation_id, &first_result_id, &second_call.id)?;

        let active_path_messages = store.load_active_path(&conversation_id)?.len();
        let tree_messages = store.load_message_tree_view(&conversation_id)?.len();

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        let duration = started.elapsed();

        Ok(RuntimeContextBenchmark {
            duration,
            active_path_messages,
            tree_messages,
            requested_tool_calls: 2,
            resolved_tool_results: 2,
        })
    })
}

/// Measures active-path loading for a generated message chain.
fn benchmark_active_path_load(message_count: usize) -> Result<Duration> {
    with_runtime_store(&format!("active-path-load-{message_count}"), |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_message_chain(store, &conversation_id, message_count)?;

        let started = Instant::now();
        let messages = store.load_active_path(&conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(messages.len(), message_count);

        Ok(duration)
    })
}

/// Measures approval scanning when a tool call appears after a long path.
fn benchmark_pending_tool_approval_scan_long_path() -> Result<Duration> {
    with_runtime_store("pending-tool-approval-long-path", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let parent_id = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
        let metadata =
            tool_call_metadata(vec![tool_call(0, "call_1", "desktop_commander__read_file")]);
        store.insert_message(
            &conversation_id,
            parent_id.as_ref(),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        let approvals = pending_tool_approvals(store, &conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(approvals.len(), 1);

        Ok(duration)
    })
}

/// Measures approval scanning after many prior tool results in one execution.
fn benchmark_pending_tool_approval_scan_deep_chain() -> Result<Duration> {
    with_runtime_store("pending-tool-approval-deep-chain", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let user_id = insert_user_message(store, &conversation_id, None, "use many tools")?;
        let tool_calls = (0..TOOL_CHAIN_RESULTS)
            .map(|index| {
                tool_call(
                    index as u16,
                    &format!("call_{index}"),
                    "desktop_commander__read_file",
                )
            })
            .collect::<Vec<_>>();
        let assistant_id = store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(tool_calls.clone())),
        )?;
        let mut parent_id = assistant_id;
        for tool_call in tool_calls.iter().take(TOOL_CHAIN_RESULTS - 1) {
            parent_id = insert_tool_result(store, &conversation_id, &parent_id, &tool_call.id)?;
        }

        let started = Instant::now();
        let approvals = pending_tool_approvals(store, &conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(approvals.len(), 1);

        Ok(duration)
    })
}

/// Measures query preparation on a plain completed active path.
fn benchmark_prepare_query_no_tools() -> Result<Duration> {
    with_runtime_store("prepare-query-no-tools", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures query preparation after all requested tool calls have results.
fn benchmark_prepare_query_completed_tool_chain() -> Result<Duration> {
    with_runtime_store("prepare-query-completed-tool-chain", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_completed_tool_chain(store, &conversation_id, TOOL_CHAIN_RESULTS)?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures the rejection path when query is waiting on approval.
fn benchmark_prepare_query_requires_approval() -> Result<Duration> {
    with_runtime_store("prepare-query-requires-approval", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let metadata =
            tool_call_metadata(vec![tool_call(0, "call_1", "desktop_commander__read_file")]);
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        let result = prepare_query_turn(store, &conversation_id);
        let duration = started.elapsed();
        debug_assert!(result.is_err());

        Ok(duration)
    })
}

/// Measures preparation when policy-denied tool calls are auto-recorded.
fn benchmark_prepare_query_policy_denied() -> Result<Duration> {
    with_runtime_store("prepare-query-policy-denied", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let metadata = tool_call_metadata(vec![tool_call(0, "denied_call", "unknown_tool")]);
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures splice delete for a branch point with many direct children.
fn benchmark_splice_remove_branch_point() -> Result<(Duration, usize, usize)> {
    with_runtime_store("splice-remove-branch-point", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let root_id = insert_user_message(store, &conversation_id, None, "root")?;
        let branch_id = store.insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "branch",
            None,
        )?;
        for index in 0..BRANCH_CHILDREN {
            insert_user_message(
                store,
                &conversation_id,
                Some(&branch_id),
                &format!("child {index}"),
            )?;
        }

        let started = Instant::now();
        store.remove_message(&conversation_id, &branch_id)?;
        Ok((started.elapsed(), 1, BRANCH_CHILDREN))
    })
}

/// Measures splice delete for a root node with many direct children.
fn benchmark_splice_remove_root_many_children() -> Result<(Duration, usize, usize)> {
    with_runtime_store("splice-remove-root-many-children", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let root_id = insert_user_message(store, &conversation_id, None, "root")?;
        for index in 0..BRANCH_CHILDREN {
            insert_user_message(
                store,
                &conversation_id,
                Some(&root_id),
                &format!("child {index}"),
            )?;
        }

        let started = Instant::now();
        store.remove_message(&conversation_id, &root_id)?;
        Ok((started.elapsed(), 1, BRANCH_CHILDREN))
    })
}

/// Measures splice delete for an assistant tool-call group.
fn benchmark_splice_remove_tool_group() -> Result<Duration> {
    with_runtime_store("splice-remove-tool-group", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let assistant_id = create_completed_tool_chain(store, &conversation_id, 2)?;
        let second_result_id = store
            .active_message_id(&conversation_id)?
            .expect("tool chain fixture should have active result");
        store.insert_message(
            &conversation_id,
            Some(&second_result_id),
            Role::Assistant,
            "done",
            None,
        )?;

        let started = Instant::now();
        store.remove_message(&conversation_id, &assistant_id)?;
        Ok(started.elapsed())
    })
}

/// Measures truncate for a subtree with many descendants.
fn benchmark_truncate_large_subtree() -> Result<(Duration, usize)> {
    with_runtime_store("truncate-large-subtree", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let checkpoint_id = insert_user_message(store, &conversation_id, None, "checkpoint")?;
        let mut parent_id = checkpoint_id.clone();
        for index in 0..LARGE_TRUNCATE_DESCENDANTS {
            parent_id = store.insert_message(
                &conversation_id,
                Some(&parent_id),
                if index % 2 == 0 {
                    Role::Assistant
                } else {
                    Role::User
                },
                &format!("descendant {index}"),
                None,
            )?;
        }

        let started = Instant::now();
        store.truncate_after_message(&conversation_id, &checkpoint_id)?;
        Ok((started.elapsed(), LARGE_TRUNCATE_DESCENDANTS))
    })
}

/// Measures context build for a plain generated message chain.
fn benchmark_context_plain(message_count: usize) -> Result<Duration> {
    with_runtime_store(&format!("context-plain-{message_count}"), |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_message_chain(store, &conversation_id, message_count)?;

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures context build when a conversation system prompt exists.
fn benchmark_context_with_system_prompt() -> Result<Duration> {
    with_runtime_store("context-with-system-prompt", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
        store.set_system_prompt(&conversation_id, "You are a concise local runtime.")?;

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures context build with a compaction summary plus a remaining suffix.
fn benchmark_context_with_compaction() -> Result<Duration> {
    with_runtime_store("context-with-compaction", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let checkpoint_index = SCALE_PATH_MESSAGES / 2;
        let mut parent_id = None;
        let mut checkpoint_id = None;

        for index in 0..SCALE_PATH_MESSAGES {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let id = store.insert_message(
                &conversation_id,
                parent_id.as_ref(),
                role,
                &format!("message {index}"),
                None,
            )?;
            if index == checkpoint_index {
                checkpoint_id = Some(id.clone());
            }
            parent_id = Some(id);
        }

        let checkpoint_id = checkpoint_id.expect("message chain fixture should have a checkpoint");
        store.save_compaction(&conversation_id, &checkpoint_id, "summary")?;

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures context build for image-heavy message parts.
fn benchmark_context_with_image_parts() -> Result<Duration> {
    with_runtime_store("context-with-image-parts", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let image_bytes = tiny_png_bytes();
        let mut parent_id = None;
        for index in 0..IMAGE_PART_MESSAGES {
            let id = store.insert_message_with_parts(
                &conversation_id,
                parent_id.as_ref(),
                Role::User,
                &format!("image message {index}"),
                &[
                    UnsavedMessagePart::Text("image".to_string()),
                    UnsavedMessagePart::Image(UnsavedImagePart {
                        mime_type: "image/png".to_string(),
                        bytes: image_bytes.to_vec(),
                    }),
                ],
                None,
            )?;
            parent_id = Some(id);
        }

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures provider registry lookup plus attached-tool persistence.
fn benchmark_provider_tool_attach_load() -> Result<Duration> {
    with_runtime_store("provider-tool-attach-load", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let definition = test_tool_definition();

        let started = Instant::now();
        store.insert_attached_tool(&conversation_id, &definition.attached_tool())?;
        let attached_tool = store
            .load_attached_tool(&conversation_id, &definition.schema_name)?
            .ok_or_else(|| anyhow::anyhow!("attached test provider tool was not stored"))?;
        let can_execute = attached_tool.provider.kind == ToolProviderKind::Mcp
            && attached_tool.provider.provider_id.as_str() == TEST_PROVIDER_ID;
        let duration = started.elapsed();
        debug_assert!(can_execute);

        Ok(duration)
    })
}

/// Measures a provider-free MCP initialize, tools/list, and tools/call path.
fn benchmark_fake_mcp_list_call() -> Result<Duration> {
    let started = Instant::now();
    let tools = mcp::list_tools(FAKE_MCP_COMMAND)?;
    let result = mcp::call_tool(FAKE_MCP_COMMAND, "click", serde_json::json!({}))?;
    let duration = started.elapsed();
    debug_assert_eq!(tools.len(), 1);
    debug_assert_eq!(tools[0].name, "click");
    debug_assert!(!result.is_error);

    Ok(duration)
}

/// Measures queued persistence after production-style delta coalescing across
/// concurrent conversations.
async fn benchmark_durable_stream_journal() -> Result<Duration> {
    let path = env::temp_dir().join(format!(
        "windie-bench-durable-stream-{}-{}.db",
        process::id(),
        Uuid::new_v4()
    ));
    let store = Store::open_at(&path)?;
    let conversation_ids = (0..4)
        .map(|_| store.create_conversation("openai/test"))
        .collect::<Result<Vec<_>>>()?;
    drop(store);

    let manager = RunManager::new(Some(path.clone()))?;
    let mut runs = Vec::new();
    for conversation_id in &conversation_ids {
        runs.push(manager.begin(conversation_id).await?);
    }
    let started = Instant::now();
    let mut writes = Vec::new();
    for run in &runs {
        writes.push(manager.enqueue(
            &run.id,
            RunEvent::AssistantDelta {
                text: "x".repeat(125),
            },
        )?);
    }
    for write in writes {
        write.persisted().await?;
    }
    for run in &runs {
        manager.complete(&run.id, None).await?;
    }
    let duration = started.elapsed();
    drop(manager);
    let _ = fs::remove_file(path);

    Ok(duration)
}

/// Measures the complete read-consistent inspector snapshot for a long path.
fn benchmark_inspection_snapshot_1000() -> Result<Duration> {
    with_runtime_store("inspection-snapshot-1000", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_message_chain(store, &conversation_id, LARGE_SCALE_PATH_MESSAGES)?;

        let started = Instant::now();
        let _ = operation::inspect_conversation(store, &conversation_id, None)?;
        Ok(started.elapsed())
    })
}

/// Measures copying a long active path into an independently owned conversation.
fn benchmark_fork_conversation_1000() -> Result<Duration> {
    with_runtime_store("fork-conversation-1000", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let checkpoint = create_message_chain(store, &conversation_id, LARGE_SCALE_PATH_MESSAGES)?
            .expect("fork benchmark fixture should contain messages");

        let started = Instant::now();
        let forked_id = store.fork_conversation_at_message(&conversation_id, &checkpoint)?;
        let duration = started.elapsed();
        debug_assert_eq!(
            store.load_active_path(&forked_id)?.len(),
            LARGE_SCALE_PATH_MESSAGES
        );
        Ok(duration)
    })
}

/// Measures begin, dedicated executor startup, and durable completion.
async fn benchmark_run_action_lifecycle() -> Result<Duration> {
    let path = runtime_database_path("run-action-lifecycle");
    let store = Store::open_at(&path)?;
    let conversation_id = store.create_conversation("openai/test")?;
    drop(store);
    let manager = RunManager::new(Some(path.clone()))?;

    let started = Instant::now();
    manager
        .execute_action(
            &conversation_id,
            RuntimeRunAction::Query,
            |_run_id, _cancellation| async { Ok(()) },
        )
        .await?;
    let duration = started.elapsed();
    drop(manager);
    remove_runtime_database_files(&path);
    Ok(duration)
}

/// Measures two simultaneous attempts to acquire one conversation run slot.
async fn benchmark_run_admission_contention() -> Result<Duration> {
    let path = runtime_database_path("run-admission-contention");
    let store = Store::open_at(&path)?;
    let conversation_id = store.create_conversation("openai/test")?;
    drop(store);
    let manager = RunManager::new(Some(path.clone()))?;

    let started = Instant::now();
    let (first, second) = tokio::join!(
        manager.begin_action(&conversation_id, RuntimeRunAction::Query),
        manager.begin_action(&conversation_id, RuntimeRunAction::Query)
    );
    let duration = started.elapsed();
    let run = match (first, second) {
        (Ok(run), Err(_)) | (Err(_), Ok(run)) => run,
        (Ok(_), Ok(_)) => anyhow::bail!("competing run admissions both succeeded"),
        (Err(first), Err(second)) => {
            return Err(anyhow::anyhow!(
                "competing run admissions both failed: {first}; {second}"
            ));
        }
    };
    manager.complete(&run.id, None).await?;
    drop(manager);
    remove_runtime_database_files(&path);
    Ok(duration)
}

/// Measures two concurrent catalog callers sharing one fake MCP process.
fn benchmark_fake_mcp_catalog_singleflight() -> Result<(Duration, usize)> {
    let nonce = Uuid::new_v4();
    let script_path = env::temp_dir().join(format!("windie-catalog-bench-{nonce}.sh"));
    let starts_path = env::temp_dir().join(format!("windie-catalog-bench-{nonce}.starts"));
    let script = format!(
        r#"#!/bin/sh
printf 'start\n' >> '{}'
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"bench","version":"1"}}}}}}'
      ;;
    *'"method":"tools/list"'*)
      sleep 0.02
      printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"search","description":"Search","inputSchema":{{"type":"object"}}}}]}}}}'
      ;;
  esac
done
"#,
        starts_path.display()
    );
    fs::write(&script_path, script)?;
    let script_argument: &'static str =
        Box::leak(script_path.to_string_lossy().into_owned().into_boxed_str());
    let arguments: &'static [&'static str] = Box::leak(vec![script_argument].into_boxed_slice());
    let registry = Arc::new(ToolProviderRegistry::with_uncached_mcp_provider(
        "catalog-benchmark",
        "catalog_benchmark",
        "Catalog Benchmark",
        crate::mcp::McpCommand {
            program: "/bin/sh",
            args: arguments,
            env: &[],
        },
    ));
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                registry.list_provider_tools(&ToolProviderId::new("catalog-benchmark"))
            })
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    barrier.wait();
    for worker in workers {
        let tools = worker
            .join()
            .map_err(|_| anyhow::anyhow!("catalog benchmark worker panicked"))??;
        debug_assert_eq!(tools.len(), 1);
    }
    let duration = started.elapsed();
    let starts = fs::read_to_string(&starts_path)?.lines().count();
    debug_assert_eq!(starts, 1);
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(starts_path);

    Ok((duration, starts))
}
