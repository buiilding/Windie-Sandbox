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
use crate::run::{RunEvent, RunManager};
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
    durable_stream_journal: Duration,
    active_path_messages: usize,
    tree_messages: usize,
    requested_tool_calls: usize,
    resolved_tool_results: usize,
    deleted_messages: usize,
    promoted_children: usize,
    truncated_messages: usize,
}

impl RuntimeBenchmarkTimings {
    fn durations(&self) -> [(MetricName, Duration); 27] {
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
            (
                MetricName::DurableStreamJournal,
                self.durable_stream_journal,
            ),
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
        BenchmarkMode::Conversation => record_conversation_benchmark(&mut baseline)?,
        BenchmarkMode::Runtime => {
            let runtime = run_runtime_benchmark().await?;
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

mod conversation;
mod fixtures;
mod live;
mod runtime;

use conversation::record_conversation_benchmark;
use fixtures::*;
use live::run_live_request;
use runtime::run_runtime_benchmark;
