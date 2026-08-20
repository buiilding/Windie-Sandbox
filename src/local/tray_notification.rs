//! Native tray notification test observer.
//!
//! This component owns only the development notification probe: it reconnects
//! to the local API's volatile test SSE stream and turns the explicit
//! assistant-completed signal into a native operating-system notification. It
//! neither starts runtime components nor reads or writes session state.

use anyhow::{Context, Result, anyhow};
use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const RECONNECT_DELAY: Duration = Duration::from_millis(250);

/// Starts the development notification observer on a dedicated native thread.
///
/// The tray event loop is synchronous, while SSE is blocking I/O. Keeping the
/// observer on its own thread makes a disconnected API harmless to tray menu
/// responsiveness and prevents HTTP runtime teardown from happening inside a
/// Tokio task.
pub fn start_test_completion_observer(stopping: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            if observe_test_notifications(&stopping).is_err() && !stopping.load(Ordering::Acquire) {
                thread::sleep(RECONNECT_DELAY);
            }
        }
    })
}

/// Holds one SSE connection open until it is interrupted, disconnects, or the
/// tray exits. The connection stays open for the lifetime of the tray, so a
/// volatile test signal cannot disappear in a reconnect gap.
fn observe_test_notifications(stopping: &AtomicBool) -> anyhow::Result<()> {
    let reader =
        crate::local::session_event_observer::open_local_sse_stream("/api/dev/tray-notifications")?;

    let mut event_name = None;
    for line in reader.lines() {
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }

        let line = line?;
        if let Some(name) = line.strip_prefix("event:") {
            event_name = Some(name.trim().to_string());
        } else if line.is_empty() {
            if is_assistant_completed_event(event_name.as_deref()) {
                eprintln!("windie tray: received assistant-completed test notification");
                if let Err(error) = show_assistant_completed("Assistant finished") {
                    eprintln!("failed to show Windie notification: {error:#}");
                }
            }
            event_name = None;
        }
    }

    Err(anyhow::anyhow!(
        "Windie API notification stream closed unexpectedly"
    ))
}

/// Opens the localhost SSE stream with bounded raw HTTP I/O.
///
/// This intentionally does not use a blocking HTTP client. The tray owns a
/// synchronous event loop, and raw bounded loopback I/O keeps its observer
/// independent from Tokio runtime lifetime rules.
/// Returns whether an SSE event is the explicit test completion signal.
fn is_assistant_completed_event(event_name: Option<&str>) -> bool {
    event_name == Some("tray.assistant_completed")
}

/// Presents a native notification containing one finished assistant response.
#[cfg(target_os = "macos")]
pub(crate) fn show_assistant_completed(content: &str) -> Result<()> {
    use std::process::Command;

    let content = apple_script_string_literal(content);
    let status = Command::new("osascript")
        .args([
            "-e",
            &format!("display notification {content} with title \"Windie\""),
        ])
        .status()
        .context("failed to run osascript for the Windie notification")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("osascript exited with {status}"))
    }
}

/// Escapes a normalized notification body for one AppleScript string literal.
#[cfg(target_os = "macos")]
fn apple_script_string_literal(content: &str) -> String {
    format!("\"{}\"", content.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Keeps the tray buildable on Windows while native Windows toast delivery is
/// intentionally deferred until the durable notification observer is added.
#[cfg(target_os = "windows")]
pub(crate) fn show_assistant_completed(content: &str) -> Result<()> {
    eprintln!("Windie tray received final assistant response: {content}");
    Ok(())
}

/// The tray itself is unavailable on these targets; keep the library's local
/// observer component portable for test and documentation builds.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn show_assistant_completed(content: &str) -> Result<()> {
    eprintln!("Windie tray received final assistant response: {content}");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn completion_event_name_matches_the_api_contract() {
        assert!(super::is_assistant_completed_event(Some(
            "tray.assistant_completed"
        )));
        assert!(!super::is_assistant_completed_event(Some(
            "session.completed"
        )));
        assert!(!super::is_assistant_completed_event(None));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apple_script_notification_body_escapes_quotes_and_backslashes() {
        let escaped = super::apple_script_string_literal(r#"quote " and path \windie"#);

        assert_eq!(escaped, r#""quote \" and path \\windie""#);
    }
}
