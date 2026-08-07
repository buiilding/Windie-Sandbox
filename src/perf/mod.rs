//! Performance measurement and comparison.
//!
//! This module owns lightweight timing for Windie's deterministic local paths,
//! repeated benchmark reports, JSON benchmark artifacts, and report comparison.
//! Named scenarios separate storage, context, serialization, runtime, session,
//! MCP, API, and lifecycle measurements. External inference and installation
//! work remains outside the default provider-free benchmark.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::{ContextBuilder, ContextParts};
use crate::conversation::{
    ConversationId, MessageId, MessageMetadata, Role, ToolCall, ToolCallId, UnsavedImagePart,
    UnsavedMessagePart,
};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, ModelName};
use crate::mcp::{self, McpCommand};
use crate::runtime::{NoopRuntimeEventSink, prepare_head_turn};
use crate::store::Store;
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef,
};
use crate::tool_provider::ToolProviderRegistry;

mod comparison;
mod fixture;
mod mode;
mod report;
mod runner;
mod runtime;
mod scenarios;
mod storage;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub use comparison::PerformanceComparisonRow;
pub use comparison::{PerformanceComparison, compare_reports};
pub use mode::{BenchmarkCategory, BenchmarkMode, BenchmarkOptions};
#[cfg(test)]
pub use report::{DurationMetric, ScenarioSummary};
pub use report::{
    PerformanceBaseline, PerformanceReport, PerformanceSample, PerformanceSummary, ScenarioTiming,
};
pub use runner::{run, run_report};
pub use storage::{default_baseline_path, read_report, write_report};

use fixture::*;
#[cfg(test)]
use report::duration_metric;
use runtime::*;

const SCALE_PATH_MESSAGES: usize = 100;

const SCALE_BRANCH_TREE_MESSAGES: usize = 1_000;

const TOOL_CHAIN_RESULTS: usize = 10;

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
    args: &[
        mcp::McpArgument::Literal("-c"),
        mcp::McpArgument::Literal(FAKE_MCP_SCRIPT),
    ],
    env: &[],
};

const REPORT_FORMAT_VERSION: u32 = 7;
