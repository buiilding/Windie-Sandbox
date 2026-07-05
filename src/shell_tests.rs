//! Tests for Windie's built-in shell executor.

use super::*;

#[tokio::test]
async fn shell_executor_captures_stdout() {
    let executor = ShellExecutor::default();
    let output = executor
        .execute(&ShellCommand {
            command: "printf hello".to_string(),
            cwd: None,
            timeout_ms: None,
        })
        .await
        .unwrap();

    assert_eq!(
        output,
        ShellOutput {
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            timed_out: false,
            duration_ms: output.duration_ms,
            stdout_truncated: false,
            stderr_truncated: false,
        }
    );
}

#[tokio::test]
async fn shell_executor_captures_stderr_and_exit_code() {
    let executor = ShellExecutor::default();
    let output = executor
        .execute(&ShellCommand {
            command: "printf problem >&2; exit 7".to_string(),
            cwd: None,
            timeout_ms: None,
        })
        .await
        .unwrap();

    assert_eq!(output.stdout, "");
    assert_eq!(output.stderr, "problem");
    assert_eq!(output.exit_code, Some(7));
    assert!(!output.timed_out);
}

#[tokio::test]
async fn shell_executor_times_out() {
    let executor = ShellExecutor::default();
    let output = executor
        .execute(&ShellCommand {
            command: "sleep 2".to_string(),
            cwd: None,
            timeout_ms: Some(1),
        })
        .await
        .unwrap();

    assert_eq!(output.exit_code, None);
    assert!(output.timed_out);
}

#[tokio::test]
async fn shell_executor_rejects_empty_command() {
    let executor = ShellExecutor::default();
    let error = executor
        .execute(&ShellCommand {
            command: "   ".to_string(),
            cwd: None,
            timeout_ms: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "shell command cannot be empty");
}
