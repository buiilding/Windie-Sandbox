//! Deterministic runtime benchmark fixtures.

use super::*;

/// Prepares the latest conversation head using the production runtime path.
fn prepare_latest_head_turn(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    let registry = ToolProviderRegistry::with_installed_plugins()?;
    let events = NoopRuntimeEventSink;
    let mut head_message_id = latest_message_id(store, conversation_id)?;

    prepare_head_turn(
        store,
        conversation_id,
        &mut head_message_id,
        &registry,
        &events,
    )
}

/// Measures explicit path loading for a generated message chain.
pub(super) fn benchmark_path_load(message_count: usize) -> Result<Duration> {
    with_runtime_store(&format!("path-load-{message_count}"), |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head_message_id = create_message_chain(store, &conversation_id, message_count)?
            .expect("message chain fixture should have a head");

        let started = Instant::now();
        let messages = store.load_path_to_message(&conversation_id, &head_message_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(messages.len(), message_count);

        Ok(duration)
    })
}

/// Measures selected-path row loading without attaching message parts.
pub(super) fn benchmark_path_rows_load(message_count: usize) -> Result<Duration> {
    with_runtime_store(&format!("path-rows-load-{message_count}"), |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head_message_id = create_message_chain(store, &conversation_id, message_count)?
            .expect("message chain fixture should have a head");

        let started = Instant::now();
        let messages = store.load_path_to_message_rows(&conversation_id, &head_message_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(messages.len(), message_count);

        Ok(duration)
    })
}

/// Measures basic row loading for a branched tree and its selected path.
pub(super) fn benchmark_branched_rows_load(selected_path: bool) -> Result<Duration> {
    with_runtime_store("branched-rows-load", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        let head = create_branched_message_tree(
            store,
            &conversation_id,
            SCALE_BRANCH_TREE_MESSAGES,
            SCALE_PATH_MESSAGES,
        )?;

        let started = Instant::now();
        if selected_path {
            let _ = store.load_path_to_message_rows(&conversation_id, &head)?;
        } else {
            let _ = store.load_message_rows(&conversation_id)?;
        }
        Ok(started.elapsed())
    })
}

/// Measures query preparation on a plain completed path.
pub(super) fn benchmark_prepare_run_head_no_tools() -> Result<Duration> {
    with_runtime_store("prepare-run-head-no-tools", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;

        let started = Instant::now();
        prepare_latest_head_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures query preparation after all requested tool calls have results.
pub(super) fn benchmark_prepare_run_head_completed_tool_chain() -> Result<Duration> {
    with_runtime_store("prepare-run-head-completed-tool-chain", |store| {
        let conversation_id = store.create_conversation("openai/test")?;
        create_completed_tool_chain(store, &conversation_id, TOOL_CHAIN_RESULTS)?;

        let started = Instant::now();
        prepare_latest_head_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures the rejection path when query is waiting on approval.
pub(super) fn benchmark_prepare_run_head_requires_approval() -> Result<Duration> {
    with_runtime_store("prepare-run-head-requires-approval", |store| {
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
        let result = prepare_latest_head_turn(store, &conversation_id);
        let duration = started.elapsed();
        let _ = result;

        Ok(duration)
    })
}

/// Measures preparation when policy-denied tool calls are auto-recorded.
pub(super) fn benchmark_prepare_run_head_policy_denied() -> Result<Duration> {
    with_runtime_store("prepare-run-head-policy-denied", |store| {
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
        prepare_latest_head_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures a provider-free MCP initialize, tools/list, and tools/call path.
pub(super) fn benchmark_fake_mcp_list_call() -> Result<Duration> {
    let started = Instant::now();
    let tools = mcp::list_tools(FAKE_MCP_COMMAND)?;
    let result = mcp::call_tool(FAKE_MCP_COMMAND, "click", serde_json::json!({}))?;
    let duration = started.elapsed();
    debug_assert_eq!(tools.len(), 1);
    debug_assert_eq!(tools[0].name, "click");
    debug_assert_eq!(result["isError"], false);

    Ok(duration)
}
