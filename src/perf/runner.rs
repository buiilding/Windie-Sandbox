//! Benchmark runner entry points.

use super::*;

/// Runs the selected benchmark mode.
///
/// Conversation mode is free/local. Live mode requires Bifrost and sends a tiny
/// real model request.
pub async fn run(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    _gateway_url: GatewayUrl,
    _base_url: BaseUrl,
    model: ModelName,
    categories: &[BenchmarkCategory],
) -> Result<PerformanceBaseline> {
    let mut baseline = PerformanceBaseline {
        mode,
        model,
        conversation_id,
        store_open: None,
        conversation_load: None,
        head_message_lookup: None,
        path_row_load: None,
        path_part_load: None,
        tree_load: None,
        tree_row_load: None,
        tree_part_load: None,
        tool_schema_load: None,
        context_build: None,
        context_path_load: None,
        context_system_prompt_load: None,
        context_compaction_load: None,
        context_flatten: None,
        prepare_head_turn: None,
        pending_tool_approval_scan: None,
        tool_result_insert: None,
        deny_tool_result_persist: None,
        splice_remove: None,
        truncate: None,
        context_build_after_tool_chain: None,
        path_load_100: None,
        path_load_1000: None,
        pending_tool_approval_scan_long_path: None,
        pending_tool_approval_scan_deep_chain: None,
        prepare_run_head_no_tools: None,
        prepare_run_head_completed_tool_chain: None,
        prepare_run_head_requires_approval: None,
        prepare_run_head_policy_denied: None,
        splice_remove_branch_point: None,
        splice_remove_root_many_children: None,
        splice_remove_tool_group: None,
        truncate_large_subtree: None,
        context_build_plain_100: None,
        context_build_plain_1000: None,
        context_build_with_system_prompt: None,
        context_build_with_compaction: None,
        context_build_with_image_parts: None,
        provider_tool_attach_load: None,
        fake_mcp_list_call: None,
        loaded_messages: None,
        tree_messages: None,
        requested_tool_calls: None,
        resolved_tool_results: None,
        deleted_messages: None,
        promoted_children: None,
        truncated_messages: None,
        gateway_ready: None,
        first_token: None,
        full_response: None,
        response_bytes: None,
    };

    match mode {
        BenchmarkMode::Conversation => {
            let store_started = Instant::now();
            let store = Store::open()?;
            let store_open = store_started.elapsed();
            let conversation_id = baseline
                .conversation_id
                .as_ref()
                .expect("conversation benchmark requires conversation id");

            let load_started = Instant::now();
            let head_lookup_started = Instant::now();
            let head_message_id = latest_message_id(&store, conversation_id)?;
            let head_message_lookup = head_lookup_started.elapsed();
            let path = if let Some(head_message_id) = head_message_id.as_ref() {
                let row_started = Instant::now();
                let messages = store.load_path_to_message_rows(conversation_id, head_message_id)?;
                let row_load = row_started.elapsed();

                let part_started = Instant::now();
                let mut messages = messages;
                store
                    .attach_message_parts(&mut messages)
                    .context("failed to load path parts")?;
                let part_load = part_started.elapsed();

                (messages, row_load, part_load)
            } else {
                (Vec::new(), Duration::ZERO, Duration::ZERO)
            };
            let loaded_messages = path.0.len();
            let path_row_load = path.1;
            let path_part_load = path.2;
            let conversation_load = load_started.elapsed();

            let tree_started = Instant::now();
            let tree_row_started = Instant::now();
            let mut tree = store.load_message_rows(conversation_id)?;
            let tree_row_load = tree_row_started.elapsed();

            let tree_part_started = Instant::now();
            store
                .attach_message_parts(&mut tree)
                .context("failed to load message tree parts")?;
            let tree_part_load = tree_part_started.elapsed();
            let tree_messages = tree.len();
            let tree_load = tree_started.elapsed();

            let tool_schema_started = Instant::now();
            let _ = store.load_tool_schemas(conversation_id)?;
            let tool_schema_load = tool_schema_started.elapsed();

            let context_started = Instant::now();
            let context_path_started = Instant::now();
            let context_path = match head_message_id.as_ref() {
                Some(head_message_id) => {
                    store.load_path_to_message(conversation_id, head_message_id)?
                }
                None => Vec::new(),
            };
            let context_path_load = context_path_started.elapsed();

            let context_system_prompt_started = Instant::now();
            let _context_system_prompt = store.system_prompt(conversation_id)?;
            let context_system_prompt_load = context_system_prompt_started.elapsed();

            let context_compaction_started = Instant::now();
            let context_compaction = store.latest_compaction(conversation_id)?;
            let context_compaction_load = context_compaction_started.elapsed();

            let context_flatten_started = Instant::now();
            let _ = ContextBuilder::flatten(ContextParts {
                path: context_path,
                system_prompt: None,
                compaction: context_compaction,
            });
            let context_flatten = context_flatten_started.elapsed();
            let context_build = context_started.elapsed();

            baseline.store_open = Some(store_open);
            baseline.conversation_load = Some(conversation_load);
            baseline.head_message_lookup = Some(head_message_lookup);
            baseline.path_row_load = Some(path_row_load);
            baseline.path_part_load = Some(path_part_load);
            baseline.tree_load = Some(tree_load);
            baseline.tree_row_load = Some(tree_row_load);
            baseline.tree_part_load = Some(tree_part_load);
            baseline.tool_schema_load = Some(tool_schema_load);
            baseline.context_build = Some(context_build);
            baseline.context_path_load = Some(context_path_load);
            baseline.context_system_prompt_load = Some(context_system_prompt_load);
            baseline.context_compaction_load = Some(context_compaction_load);
            baseline.context_flatten = Some(context_flatten);
            baseline.loaded_messages = Some(loaded_messages);
            baseline.tree_messages = Some(tree_messages);
        }
        BenchmarkMode::Local => {
            let runtime = run_runtime_benchmark()?;
            baseline.prepare_head_turn = Some(runtime.prepare_head_turn);
            baseline.pending_tool_approval_scan = Some(runtime.pending_tool_approval_scan);
            baseline.tool_result_insert = Some(runtime.tool_result_insert);
            baseline.deny_tool_result_persist = Some(runtime.deny_tool_result_persist);
            baseline.splice_remove = Some(runtime.splice_remove);
            baseline.truncate = Some(runtime.truncate);
            baseline.context_build_after_tool_chain = Some(runtime.context_build_after_tool_chain);
            baseline.path_load_100 = Some(runtime.path_load_100);
            baseline.path_load_1000 = Some(runtime.path_load_1000);
            baseline.pending_tool_approval_scan_long_path =
                Some(runtime.pending_tool_approval_scan_long_path);
            baseline.pending_tool_approval_scan_deep_chain =
                Some(runtime.pending_tool_approval_scan_deep_chain);
            baseline.prepare_run_head_no_tools = Some(runtime.prepare_run_head_no_tools);
            baseline.prepare_run_head_completed_tool_chain =
                Some(runtime.prepare_run_head_completed_tool_chain);
            baseline.prepare_run_head_requires_approval =
                Some(runtime.prepare_run_head_requires_approval);
            baseline.prepare_run_head_policy_denied = Some(runtime.prepare_run_head_policy_denied);
            baseline.splice_remove_branch_point = Some(runtime.splice_remove_branch_point);
            baseline.splice_remove_root_many_children =
                Some(runtime.splice_remove_root_many_children);
            baseline.splice_remove_tool_group = Some(runtime.splice_remove_tool_group);
            baseline.truncate_large_subtree = Some(runtime.truncate_large_subtree);
            baseline.context_build_plain_100 = Some(runtime.context_build_plain_100);
            baseline.context_build_plain_1000 = Some(runtime.context_build_plain_1000);
            baseline.context_build_with_system_prompt =
                Some(runtime.context_build_with_system_prompt);
            baseline.context_build_with_compaction = Some(runtime.context_build_with_compaction);
            baseline.context_build_with_image_parts = Some(runtime.context_build_with_image_parts);
            baseline.provider_tool_attach_load = Some(runtime.provider_tool_attach_load);
            baseline.fake_mcp_list_call = Some(runtime.fake_mcp_list_call);
            baseline.loaded_messages = Some(runtime.path_messages);
            baseline.tree_messages = Some(runtime.tree_messages);
            baseline.requested_tool_calls = Some(runtime.requested_tool_calls);
            baseline.resolved_tool_results = Some(runtime.resolved_tool_results);
            baseline.deleted_messages = Some(runtime.deleted_messages);
            baseline.promoted_children = Some(runtime.promoted_children);
            baseline.truncated_messages = Some(runtime.truncated_messages);
            apply_benchmark_categories(&mut baseline, categories);
        }
    }

    Ok(baseline)
}

/// Clears metrics outside the selected local benchmark categories.
pub(super) fn apply_benchmark_categories(
    baseline: &mut PerformanceBaseline,
    categories: &[BenchmarkCategory],
) {
    if !categories.contains(&BenchmarkCategory::Runtime) {
        baseline.prepare_head_turn = None;
        baseline.prepare_run_head_no_tools = None;
        baseline.prepare_run_head_completed_tool_chain = None;
        baseline.prepare_run_head_requires_approval = None;
        baseline.prepare_run_head_policy_denied = None;
    }
    if !categories.contains(&BenchmarkCategory::Tools) {
        baseline.pending_tool_approval_scan = None;
        baseline.pending_tool_approval_scan_long_path = None;
        baseline.pending_tool_approval_scan_deep_chain = None;
        baseline.tool_result_insert = None;
        baseline.deny_tool_result_persist = None;
        baseline.provider_tool_attach_load = None;
        baseline.requested_tool_calls = None;
        baseline.resolved_tool_results = None;
    }
    if !categories.contains(&BenchmarkCategory::Mutations) {
        baseline.splice_remove = None;
        baseline.splice_remove_branch_point = None;
        baseline.splice_remove_root_many_children = None;
        baseline.splice_remove_tool_group = None;
        baseline.truncate = None;
        baseline.truncate_large_subtree = None;
        baseline.deleted_messages = None;
        baseline.promoted_children = None;
        baseline.truncated_messages = None;
    }
    if !categories.contains(&BenchmarkCategory::Persistence) {
        baseline.path_load_100 = None;
        baseline.path_load_1000 = None;
        baseline.loaded_messages = None;
        baseline.tree_messages = None;
    }
    if !categories.contains(&BenchmarkCategory::Conversation) {
        baseline.context_build_after_tool_chain = None;
        baseline.context_build_plain_100 = None;
        baseline.context_build_plain_1000 = None;
        baseline.context_build_with_system_prompt = None;
        baseline.context_build_with_compaction = None;
        baseline.context_build_with_image_parts = None;
    }
    if !categories.contains(&BenchmarkCategory::Mcp) {
        baseline.fake_mcp_list_call = None;
    }
}

/// Runs the selected benchmark repeatedly and returns a persistent report.
pub async fn run_report(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
    options: &BenchmarkOptions,
) -> Result<PerformanceReport> {
    let runs = options.runs;
    let mut samples = Vec::with_capacity(runs);

    for _ in 0..runs {
        let baseline = run(
            mode,
            conversation_id.clone(),
            gateway_url.clone(),
            base_url.clone(),
            model.clone(),
            &options.categories,
        )
        .await?;
        samples.push(PerformanceSample::from_baseline(&baseline));
    }

    Ok(PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode,
        categories: options.categories.clone(),
        model: model.as_str().to_string(),
        conversation_id: conversation_id.map(|id| id.as_str().to_string()),
        runs,
        summary: PerformanceSummary::from_samples(&samples),
        samples,
    })
}
