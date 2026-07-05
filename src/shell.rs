//! Built-in shell execution tool.
//!
//! This module owns Windie's first native tool executor: `run_shell`. It turns
//! a model-requested shell command into a bounded local process execution and
//! returns a JSON result that can be stored as a tool message.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::conversation::ToolCall;
use crate::tool::ToolExecutionResult;

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_OUTPUT_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// JSON arguments accepted by the built-in `run_shell` tool.
pub struct ShellCommand {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Bounded shell result serialized into the tool message content.
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, Copy)]
/// Executor for Windie's built-in `run_shell` tool.
pub struct ShellExecutor {
    output_max_bytes: usize,
}

impl Default for ShellExecutor {
    fn default() -> Self {
        Self {
            output_max_bytes: DEFAULT_OUTPUT_MAX_BYTES,
        }
    }
}

impl ShellExecutor {
    /// Executes a `run_shell` tool call and returns model-facing JSON output.
    pub async fn execute_tool_call(&self, tool_call: &ToolCall) -> ToolExecutionResult {
        let result = match ShellCommand::from_tool_call(tool_call) {
            Ok(command) => self.execute(&command).await,
            Err(error) => Err(error),
        };

        match result {
            Ok(output) => ToolExecutionResult::success(
                tool_call.id.clone(),
                tool_call.name(),
                serde_json::to_string(&output).unwrap_or_else(|error| {
                    format!(r#"{{"error":"failed to encode shell output: {error}"}}"#)
                }),
            ),
            Err(error) => ToolExecutionResult::failure(
                tool_call.id.clone(),
                tool_call.name(),
                error.to_string(),
            ),
        }
    }

    /// Runs one shell command with timeout and output caps.
    pub async fn execute(&self, command: &ShellCommand) -> Result<ShellOutput> {
        command.validate()?;

        let started = Instant::now();
        let timeout_ms = command
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
        let mut process = Command::new(default_shell());
        process
            .arg("-lc")
            .arg(&command.command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(cwd) = command.cwd.as_ref() {
            process.current_dir(cwd);
        }

        let child = process.spawn().context("failed to start shell command")?;
        let timed =
            tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await;

        match timed {
            Ok(output) => {
                let output = output.context("failed to wait for shell command")?;
                let (stdout, stdout_truncated) =
                    capped_utf8_lossy(&output.stdout, self.output_max_bytes);
                let (stderr, stderr_truncated) =
                    capped_utf8_lossy(&output.stderr, self.output_max_bytes);

                Ok(ShellOutput {
                    stdout,
                    stderr,
                    exit_code: output.status.code(),
                    timed_out: false,
                    duration_ms: started.elapsed().as_millis(),
                    stdout_truncated,
                    stderr_truncated,
                })
            }
            Err(_) => Ok(ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                timed_out: true,
                duration_ms: started.elapsed().as_millis(),
                stdout_truncated: false,
                stderr_truncated: false,
            }),
        }
    }
}

impl ShellCommand {
    /// Parses `run_shell` arguments from a model tool call.
    fn from_tool_call(tool_call: &ToolCall) -> Result<Self> {
        if tool_call.name() != "run_shell" {
            return Err(anyhow!(
                "shell executor cannot run tool: {}",
                tool_call.name()
            ));
        }

        serde_json::from_str(tool_call.arguments()).context("failed to parse run_shell arguments")
    }

    /// Validates required command fields before process spawn.
    fn validate(&self) -> Result<()> {
        if self.command.trim().is_empty() {
            return Err(anyhow!("shell command cannot be empty"));
        }

        Ok(())
    }
}

/// Returns the shell Windie uses for AI-facing shell command strings.
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Converts bytes to UTF-8 text and truncates on a valid character boundary.
fn capped_utf8_lossy(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    let mut text = String::from_utf8_lossy(bytes).to_string();
    if text.len() <= max_bytes {
        return (text, false);
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n[truncated]");

    (text, true)
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
