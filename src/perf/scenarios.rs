//! Conversation, runtime, MCP, and live-provider benchmark scenarios.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use uuid::Uuid;

use super::metrics::REPORT_FORMAT_VERSION;
use super::*;
use crate::context::{ContextBuilder, ContextParts};
use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, Role, ToolCall, ToolCallId,
    UnsavedImagePart, UnsavedMessagePart,
};
use crate::gateway::{BifrostGateway, GatewayUrl};
use crate::llm::{BaseUrl, BifrostClient, LlmStreamEvent, ModelName};
use crate::mcp::{self, McpCommand};
use crate::runtime::{deny_tool_call, pending_tool_approvals, prepare_query_turn};
use crate::store::Store;
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef,
};

const BENCH_PROMPT: &str = "Reply with exactly: ok";
const SCALE_PATH_MESSAGES: usize = 100;
const LARGE_SCALE_PATH_MESSAGES: usize = 1_000;
const TOOL_CHAIN_RESULTS: usize = 10;
const BRANCH_CHILDREN: usize = 100;
const LARGE_TRUNCATE_DESCENDANTS: usize = 100;
const IMAGE_PART_MESSAGES: usize = 10;
const TEST_PROVIDER_ID: &str = "desktop-commander";
const TEST_PROVIDER_TOOL_NAME: &str = "read_file";
const TEST_TOOL_SCHEMA_NAME: &str = "desktop_commander__read_file";
const FAKE_MCP_SCRIPT: &str = r#"while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"windie-fake-mcp","version":"0"}}}' ;;
*'"method":"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"click","description":"Fake click","inputSchema":{"type":"object","additionalProperties":false,"properties":{}}}]}}' ;;
*'"method":"tools/call"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}' ;;
esac
done"#;
const FAKE_MCP_COMMAND: McpCommand = McpCommand {
    program: "/bin/sh",
    args: &["-c", FAKE_MCP_SCRIPT],
    env: &[],
};

/// Provider-free timings for runtime and write-path primitives.
struct RuntimeBenchmarkTimings {
    prepare_query_turn: Duration,
    pending_tool_approval_scan: Duration,
    tool_result_insert: Duration,
    deny_tool_result_persist: Duration,
    splice_remove: Duration,
    truncate: Duration,
    context_build_after_tool_chain: Duration,
    active_path_load_100: Duration,
    active_path_load_1000: Duration,
    pending_tool_approval_scan_long_path: Duration,
    pending_tool_approval_scan_deep_chain: Duration,
    prepare_query_no_tools: Duration,
    prepare_query_completed_tool_chain: Duration,
    prepare_query_requires_approval: Duration,
    prepare_query_policy_denied: Duration,
    splice_remove_branch_point: Duration,
    splice_remove_root_many_children: Duration,
    splice_remove_tool_group: Duration,
    truncate_large_subtree: Duration,
    context_build_plain_100: Duration,
    context_build_plain_1000: Duration,
    context_build_with_system_prompt: Duration,
    context_build_with_compaction: Duration,
    context_build_with_image_parts: Duration,
    provider_tool_attach_load: Duration,
    fake_mcp_list_call: Duration,
    active_path_messages: usize,
    tree_messages: usize,
    requested_tool_calls: usize,
    resolved_tool_results: usize,
    deleted_messages: usize,
    promoted_children: usize,
    truncated_messages: usize,
}

impl RuntimeBenchmarkTimings {
    fn durations(&self) -> [(MetricName, Duration); 26] {
        [
            (MetricName::PrepareQueryTurn, self.prepare_query_turn),
            (
                MetricName::PendingToolApprovalScan,
                self.pending_tool_approval_scan,
            ),
            (MetricName::ToolResultInsert, self.tool_result_insert),
            (
                MetricName::DenyToolResultPersist,
                self.deny_tool_result_persist,
            ),
            (MetricName::SpliceRemove, self.splice_remove),
            (MetricName::Truncate, self.truncate),
            (
                MetricName::ContextBuildAfterToolChain,
                self.context_build_after_tool_chain,
            ),
            (MetricName::ActivePathLoad100, self.active_path_load_100),
            (MetricName::ActivePathLoad1000, self.active_path_load_1000),
            (
                MetricName::PendingToolApprovalScanLongPath,
                self.pending_tool_approval_scan_long_path,
            ),
            (
                MetricName::PendingToolApprovalScanDeepChain,
                self.pending_tool_approval_scan_deep_chain,
            ),
            (MetricName::PrepareQueryNoTools, self.prepare_query_no_tools),
            (
                MetricName::PrepareQueryCompletedToolChain,
                self.prepare_query_completed_tool_chain,
            ),
            (
                MetricName::PrepareQueryRequiresApproval,
                self.prepare_query_requires_approval,
            ),
            (
                MetricName::PrepareQueryPolicyDenied,
                self.prepare_query_policy_denied,
            ),
            (
                MetricName::SpliceRemoveBranchPoint,
                self.splice_remove_branch_point,
            ),
            (
                MetricName::SpliceRemoveRootManyChildren,
                self.splice_remove_root_many_children,
            ),
            (
                MetricName::SpliceRemoveToolGroup,
                self.splice_remove_tool_group,
            ),
            (
                MetricName::TruncateLargeSubtree,
                self.truncate_large_subtree,
            ),
            (
                MetricName::ContextBuildPlain100,
                self.context_build_plain_100,
            ),
            (
                MetricName::ContextBuildPlain1000,
                self.context_build_plain_1000,
            ),
            (
                MetricName::ContextBuildWithSystemPrompt,
                self.context_build_with_system_prompt,
            ),
            (
                MetricName::ContextBuildWithCompaction,
                self.context_build_with_compaction,
            ),
            (
                MetricName::ContextBuildWithImageParts,
                self.context_build_with_image_parts,
            ),
            (
                MetricName::ProviderToolAttachLoad,
                self.provider_tool_attach_load,
            ),
            (MetricName::FakeMcpListCall, self.fake_mcp_list_call),
        ]
    }

    fn counts(&self) -> [(CountName, usize); 7] {
        [
            (CountName::ActivePathMessages, self.active_path_messages),
            (CountName::TreeMessages, self.tree_messages),
            (CountName::RequestedToolCalls, self.requested_tool_calls),
            (CountName::ResolvedToolResults, self.resolved_tool_results),
            (CountName::DeletedMessages, self.deleted_messages),
            (CountName::PromotedChildren, self.promoted_children),
            (CountName::TruncatedMessages, self.truncated_messages),
        ]
    }
}

/// Counts and duration from the context-after-tool-chain scenario.
struct RuntimeContextBenchmark {
    duration: Duration,
    active_path_messages: usize,
    tree_messages: usize,
    requested_tool_calls: usize,
    resolved_tool_results: usize,
}

/// Runs the selected benchmark mode.
///
/// Conversation mode is free/local. Live mode requires Bifrost and sends a tiny
/// real model request.
pub async fn run(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
) -> Result<PerformanceBaseline> {
    let mut baseline = PerformanceBaseline {
        mode,
        model,
        conversation_id,
        durations: BTreeMap::new(),
        counts: BTreeMap::new(),
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
            let active_message_lookup_started = Instant::now();
            let active_message_id = store.active_message_id(conversation_id)?;
            let active_message_lookup = active_message_lookup_started.elapsed();
            let active_path = if let Some(active_message_id) = active_message_id.as_ref() {
                let row_started = Instant::now();
                let messages =
                    store.load_path_to_message_rows(conversation_id, active_message_id)?;
                let row_load = row_started.elapsed();

                let part_started = Instant::now();
                let mut messages = messages;
                store
                    .attach_message_parts(&mut messages)
                    .context("failed to load active path parts")?;
                let part_load = part_started.elapsed();

                (messages, row_load, part_load)
            } else {
                (Vec::new(), Duration::ZERO, Duration::ZERO)
            };
            let loaded_messages = active_path.0.len();
            let active_path_row_load = active_path.1;
            let active_path_part_load = active_path.2;
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
            let context_active_path_started = Instant::now();
            let context_active_path = store.load_active_path(conversation_id)?;
            let context_active_path_load = context_active_path_started.elapsed();

            let context_system_prompt_started = Instant::now();
            let context_system_prompt = store.system_prompt(conversation_id)?;
            let context_system_prompt_load = context_system_prompt_started.elapsed();

            let context_compaction_started = Instant::now();
            let context_compaction = store.latest_compaction(conversation_id)?;
            let context_compaction_load = context_compaction_started.elapsed();

            let context_flatten_started = Instant::now();
            let _ = ContextBuilder::flatten(ContextParts {
                active_path: context_active_path,
                system_prompt: context_system_prompt,
                compaction: context_compaction,
            });
            let context_flatten = context_flatten_started.elapsed();
            let context_build = context_started.elapsed();

            baseline.record(MetricName::StoreOpen, store_open);
            baseline.record(MetricName::ActivePathLoad, conversation_load);
            baseline.record(MetricName::ActiveMessageLookup, active_message_lookup);
            baseline.record(MetricName::ActivePathRowLoad, active_path_row_load);
            baseline.record(MetricName::ActivePathPartLoad, active_path_part_load);
            baseline.record(MetricName::TreeLoad, tree_load);
            baseline.record(MetricName::TreeRowLoad, tree_row_load);
            baseline.record(MetricName::TreePartLoad, tree_part_load);
            baseline.record(MetricName::ToolSchemaLoad, tool_schema_load);
            baseline.record(MetricName::ContextBuild, context_build);
            baseline.record(MetricName::ContextActivePathLoad, context_active_path_load);
            baseline.record(
                MetricName::ContextSystemPromptLoad,
                context_system_prompt_load,
            );
            baseline.record(MetricName::ContextCompactionLoad, context_compaction_load);
            baseline.record(MetricName::ContextFlatten, context_flatten);
            baseline.count(CountName::ActivePathMessages, loaded_messages);
            baseline.count(CountName::TreeMessages, tree_messages);
        }
        BenchmarkMode::Runtime => {
            let runtime = run_runtime_benchmark()?;
            for (name, value) in runtime.durations() {
                baseline.record(name, value);
            }
            for (name, value) in runtime.counts() {
                baseline.count(name, value);
            }
        }
        BenchmarkMode::Live => {
            let gateway = BifrostGateway::new(gateway_url);
            let gateway_started = Instant::now();
            gateway.require_running().await?;
            baseline.record(MetricName::GatewayReady, gateway_started.elapsed());
            let (first_token, full_response, response_bytes) =
                run_live_request(&base_url, &baseline.model).await?;
            baseline.record_optional(MetricName::FirstToken, first_token);
            baseline.record(MetricName::FullResponse, full_response);
            baseline.count(CountName::ResponseBytes, response_bytes);
        }
    }

    Ok(baseline)
}

/// Runs the selected benchmark repeatedly and returns a persistent report.
pub async fn run_report(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
    runs: usize,
) -> Result<PerformanceReport> {
    let mut samples = Vec::with_capacity(runs);

    for _ in 0..runs {
        let baseline = run(
            mode,
            conversation_id.clone(),
            gateway_url.clone(),
            base_url.clone(),
            model.clone(),
        )
        .await?;
        samples.push(PerformanceSample::from_baseline(&baseline));
    }

    Ok(PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode,
        model: model.as_str().to_string(),
        conversation_id: conversation_id.map(|id| id.as_str().to_string()),
        runs,
        summary: PerformanceSummary::from_samples(&samples),
        samples,
    })
}

/// primitive costs rather than fixture construction.
fn run_runtime_benchmark() -> Result<RuntimeBenchmarkTimings> {
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
        active_path_messages: context.active_path_messages,
        tree_messages: context.tree_messages,
        requested_tool_calls: context.requested_tool_calls,
        resolved_tool_results: context.resolved_tool_results,
        deleted_messages,
        promoted_children,
        truncated_messages,
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
        let tree_messages = store.load_message_tree(&conversation_id)?.len();

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
    debug_assert_eq!(result["isError"], false);

    Ok(duration)
}

/// Creates one temporary benchmark store and removes database files after use.
fn with_runtime_store<T>(scenario: &str, run: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
    let path = runtime_database_path(scenario);
    let result = {
        let mut store = Store::open_at(&path)?;
        run(&mut store)
    };
    remove_runtime_database_files(&path);

    result
}

/// Builds a unique SQLite path for one runtime benchmark scenario.
fn runtime_database_path(scenario: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "windie-runtime-bench-{scenario}-{}-{}.db",
        process::id(),
        Uuid::new_v4()
    ))
}

/// Removes SQLite database files created for one benchmark scenario.
fn remove_runtime_database_files(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("db-wal"));
    let _ = fs::remove_file(path.with_extension("db-shm"));
}

/// Inserts a simple user message and returns its generated message ID.
fn insert_user_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    content: &str,
) -> Result<MessageId> {
    store.insert_message(
        conversation_id,
        parent_message_id,
        Role::User,
        content,
        None,
    )
}

/// Attaches a test MCP provider tool to a benchmark conversation.
///
/// Runtime approval benchmarks need the same provider-backed attachment that
/// real conversations use; otherwise policy would measure the detached-tool
/// denial path instead of the approval path.
fn attach_test_mcp_tool(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    store.insert_attached_tool(conversation_id, &test_tool_definition().attached_tool())
}

/// Builds the provider-backed test tool used by runtime benchmarks.
fn test_tool_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: crate::conversation::ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
        display_name: "Desktop Commander read_file".to_string(),
        description: "Read a file through Desktop Commander.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new(TEST_PROVIDER_ID),
            ProviderToolName::new(TEST_PROVIDER_TOOL_NAME),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

/// Creates a linear active path with alternating user and assistant messages.
fn create_message_chain(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_count: usize,
) -> Result<Option<MessageId>> {
    let mut parent_id = None;

    for index in 0..message_count {
        let role = if index % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        let id = store.insert_message(
            conversation_id,
            parent_id.as_ref(),
            role,
            &format!("message {index}"),
            None,
        )?;
        parent_id = Some(id);
    }

    Ok(parent_id)
}

/// Creates one assistant tool-call message with all requested results stored.
fn create_completed_tool_chain(
    store: &mut Store,
    conversation_id: &ConversationId,
    result_count: usize,
) -> Result<MessageId> {
    let user_id = insert_user_message(store, conversation_id, None, "use tools")?;
    let tool_calls = (0..result_count)
        .map(|index| {
            tool_call(
                index as u16,
                &format!("call_{index}"),
                "desktop_commander__read_file",
            )
        })
        .collect::<Vec<_>>();
    let assistant_id = store.insert_message(
        conversation_id,
        Some(&user_id),
        Role::Assistant,
        "",
        Some(&tool_call_metadata(tool_calls.clone())),
    )?;
    let mut parent_id = assistant_id.clone();
    for tool_call in &tool_calls {
        parent_id = insert_tool_result(store, conversation_id, &parent_id, &tool_call.id)?;
    }

    Ok(assistant_id)
}

/// Inserts one model-facing tool result under the requested chain parent.
fn insert_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &ToolCallId,
) -> Result<MessageId> {
    store.insert_tool_result_message(
        conversation_id,
        parent_message_id,
        tool_call_id,
        r#"{"stdout":"ok","stderr":"","exit_code":0}"#,
    )
}

/// Builds assistant metadata for requested tool calls.
fn tool_call_metadata(tool_calls: Vec<ToolCall>) -> MessageMetadata {
    MessageMetadata {
        tool_calls,
        ..Default::default()
    }
}

/// Builds a deterministic function tool call for runtime benchmark fixtures.
fn tool_call(index: u16, id: &str, name: &str) -> ToolCall {
    let mut tool_call = ToolCall::function(id, name, r#"{"command":"printf ok"}"#);
    tool_call.index = index;
    tool_call
}

/// Returns tiny deterministic bytes for image-part benchmark fixtures.
fn tiny_png_bytes() -> &'static [u8] {
    &[
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0, b'I', b'E', b'N', b'D',
    ]
}

/// Sends the tiny live request and measures first-token and full-response
/// latency.
async fn run_live_request(
    base_url: &BaseUrl,
    model: &ModelName,
) -> Result<(Option<Duration>, Duration, usize)> {
    let llm = BifrostClient::new(base_url.clone(), model.clone());
    let messages = vec![Message {
        id: None,
        parent_message_id: None,
        role: Role::User,
        content: BENCH_PROMPT.to_string(),
        parts: Vec::new(),
        metadata: None,
    }];

    let request_started = Instant::now();
    let mut first_token = None;
    let response = llm
        .stream(&messages, &[], None, None, |event| {
            let LlmStreamEvent::AssistantDelta(delta) = event else {
                return Ok(());
            };

            if first_token.is_none() && !delta.is_empty() {
                first_token = Some(request_started.elapsed());
            }

            Ok(())
        })
        .await?;
    let full_response = request_started.elapsed();

    Ok((first_token, full_response, response.content.len()))
}
