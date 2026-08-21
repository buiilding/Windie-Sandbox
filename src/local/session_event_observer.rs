//! Durable session-completion observation for the native notifier.
//!
//! The notifier is presentation only. This component reconnects to Windie's
//! database-backed aggregate session-event stream, persists the last displayed
//! completion cursor, and forwards only final `session.completed` events to
//! the native notification presenter. It never runs a model, changes a
//! session, or infers completion from assistant or tool messages.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

const RECONNECT_DELAY: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const COMPLETION_CURSOR_FILE_NAME: &str = "notifier-completed-event.cursor";
const COMPLETION_EVENT_NAME: &str = "session.completed";
const MAX_NOTIFICATION_BODY_CHARS: usize = 240;

#[derive(Debug, Deserialize)]
/// API response used to establish the notifier's first durable replay cursor.
struct CompletionCursorResponse {
    latest_event_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
/// Minimal aggregate event projection required to show a final response.
struct CompletionEventData {
    session_id: String,
    message: Option<CompletionMessage>,
}

#[derive(Debug, Deserialize)]
/// Canonical final assistant message projected by the aggregate event feed.
struct CompletionMessage {
    content: String,
}

/// Starts the durable final-response observer on its own native thread.
///
/// On its first connection it deliberately begins after existing completed
/// sessions, so installing or restarting the notifier does not produce a burst of
/// stale notifications. Every later reconnect resumes after the persisted
/// cursor and therefore replays any completion that the notifier has not displayed.
pub fn start_completed_session_observer(stopping: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut cursor = None;

        while !stopping.load(Ordering::Acquire) {
            if cursor.is_none() {
                match initialize_completion_cursor() {
                    Ok(initialized) => cursor = Some(initialized),
                    Err(error) => {
                        eprintln!(
                            "windie notifier: failed to initialize completion cursor: {error:#}"
                        );
                        thread::sleep(RECONNECT_DELAY);
                    }
                }
                continue;
            }

            if observe_completed_sessions(
                cursor.as_mut().expect("cursor is initialized"),
                &stopping,
            )
            .is_err()
                && !stopping.load(Ordering::Acquire)
            {
                thread::sleep(RECONNECT_DELAY);
            }
        }
    })
}

/// Watches one aggregate SSE connection for final session-completion events.
fn observe_completed_sessions(cursor: &mut i64, stopping: &AtomicBool) -> Result<()> {
    let reader = open_local_sse_stream(&completion_stream_path(*cursor))?;
    let mut event_name = None;
    let mut event_id = None;
    let mut data = Vec::new();

    for line in reader.lines() {
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }

        let line = line?;
        if let Some(name) = line.strip_prefix("event:") {
            event_name = Some(name.trim().to_string());
        } else if let Some(id) = line.strip_prefix("id:") {
            event_id = id.trim().parse::<i64>().ok();
        } else if let Some(event_data) = line.strip_prefix("data:") {
            data.push(event_data.trim_start().to_string());
        } else if line.is_empty() {
            if event_name.as_deref() == Some(COMPLETION_EVENT_NAME)
                && let Some(id) = event_id.filter(|id| *id > *cursor)
            {
                let content = data.join("\n");
                match completion_notification(&content) {
                    Some(notification) => {
                        eprintln!("windie notifier: received final assistant response");
                        crate::local::tray_notification::show_assistant_completed(
                            &notification.body,
                            &notification.session_id,
                        )?;
                        save_completion_cursor(id)?;
                        *cursor = id;
                    }
                    None => {
                        eprintln!(
                            "windie notifier: completed session {id} had no presentable final response"
                        );
                        save_completion_cursor(id)?;
                        *cursor = id;
                    }
                }
            }
            event_name = None;
            event_id = None;
            data.clear();
        }
    }

    Err(anyhow!("Windie API completion stream closed unexpectedly"))
}

/// Opens one local SSE stream through bounded loopback connection setup.
///
/// The returned connection intentionally has no read timeout: an SSE stream is
/// expected to remain quiet until a final completion happens. The notifier
/// exits independently, so its worker thread does not delay notifier shutdown.
pub(crate) fn open_local_sse_stream(path: &str) -> Result<BufReader<TcpStream>> {
    open_local_http_stream(path, "text/event-stream", "keep-alive")
}

/// Opens one local GET request with an explicit response lifetime.
fn open_local_http_stream(
    path: &str,
    accept: &str,
    connection: &str,
) -> Result<BufReader<TcpStream>> {
    let address = crate::config::api_address();
    let socket = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("could not resolve Windie API address {address}"))?;
    let mut stream = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT)?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    stream.write_all(
        format!(
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: {accept}\r\nConnection: {connection}\r\n\r\n"
        )
        .as_bytes(),
    )?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    if !status_line.starts_with("HTTP/1.1 200") {
        return Err(anyhow!(
            "Windie API SSE stream returned {}",
            status_line.trim()
        ));
    }

    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header == "\r\n" || header.is_empty() {
            return Ok(reader);
        }
    }
}

/// Builds the only aggregate event request that can generate a final-response
/// notification.
fn completion_stream_path(cursor: i64) -> String {
    format!("/api/events?after={cursor}&kind=completed")
}

/// One presentable completion notification with its durable session identity.
struct CompletionNotification {
    session_id: crate::session::SessionId,
    body: String,
}

/// Extracts an OS-notification-safe preview and durable session identity from
/// a completed event.
///
/// Native notification centres truncate long text inconsistently. Normalizing
/// whitespace and applying one explicit Unicode-safe limit keeps the notifier
/// surface readable while preserving the beginning of the actual response.
fn completion_notification(event_data: &str) -> Option<CompletionNotification> {
    let event = serde_json::from_str::<CompletionEventData>(event_data).ok()?;
    Some(CompletionNotification {
        session_id: crate::session::SessionId::new(event.session_id),
        body: notification_preview(&event.message?.content)?,
    })
}

/// Produces the native notification body for one final assistant response.
fn notification_preview(content: &str) -> Option<String> {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }

    let mut characters = normalized.chars();
    let preview = characters
        .by_ref()
        .take(MAX_NOTIFICATION_BODY_CHARS)
        .collect::<String>();
    Some(if characters.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    })
}

/// Returns the local file holding the last completion shown by the notifier.
fn completion_cursor_path() -> Result<PathBuf> {
    Ok(crate::local::windie_home_dir()?.join(COMPLETION_CURSOR_FILE_NAME))
}

/// Loads an optional durable completion cursor. A missing file is the notifier's
/// first-run state, not an error.
fn load_completion_cursor() -> Result<Option<i64>> {
    load_cursor_at(&completion_cursor_path()?)
}

/// Establishes the cursor boundary before the first live stream connects.
///
/// This snapshot-and-subscribe sequence means a completion inserted between
/// the cursor request and the SSE connection is replayed rather than skipped.
fn initialize_completion_cursor() -> Result<i64> {
    match load_completion_cursor()? {
        Some(cursor) => Ok(cursor),
        None => {
            let cursor = latest_completed_event_cursor()?;
            save_completion_cursor(cursor)?;
            Ok(cursor)
        }
    }
}

/// Reads the API's current durable completion cursor without consuming events.
fn latest_completed_event_cursor() -> Result<i64> {
    let mut reader = open_local_http_stream(
        "/api/events/cursor?kind=completed",
        "application/json",
        "close",
    )?;
    let mut body = String::new();
    reader
        .read_to_string(&mut body)
        .context("failed to read Windie API completion cursor response")?;
    Ok(serde_json::from_str::<CompletionCursorResponse>(&body)
        .context("failed to decode Windie API completion cursor response")?
        .latest_event_id
        .unwrap_or(0))
}

fn load_cursor_at(path: &Path) -> Result<Option<i64>> {
    if !path.is_file() {
        return Ok(None);
    }

    fs::read_to_string(path)
        .with_context(|| {
            format!(
                "failed to read notifier completion cursor {}",
                path.display()
            )
        })?
        .trim()
        .parse::<i64>()
        .map(Some)
        .with_context(|| format!("invalid notifier completion cursor {}", path.display()))
}

/// Atomically records a displayed completion before a reconnect can replay it.
fn save_completion_cursor(cursor: i64) -> Result<()> {
    save_cursor_at(&completion_cursor_path()?, cursor)
}

fn save_cursor_at(path: &Path, cursor: i64) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("notifier completion cursor has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create notifier cursor directory {}",
            parent.display()
        )
    })?;
    let temporary = path.with_extension("cursor.tmp");
    fs::write(&temporary, format!("{cursor}\n")).with_context(|| {
        format!(
            "failed to write notifier completion cursor {}",
            temporary.display()
        )
    })?;
    fs::rename(&temporary, path).with_context(|| {
        format!(
            "failed to publish notifier completion cursor {}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static CURSOR_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn completion_stream_filters_to_only_final_session_events() {
        assert_eq!(
            completion_stream_path(42),
            "/api/events?after=42&kind=completed"
        );
    }

    #[test]
    fn completion_cursor_response_uses_zero_when_no_final_response_exists() {
        let empty: CompletionCursorResponse =
            serde_json::from_str(r#"{"latest_event_id":null}"#).unwrap();
        let completed: CompletionCursorResponse =
            serde_json::from_str(r#"{"latest_event_id":42}"#).unwrap();

        assert_eq!(empty.latest_event_id.unwrap_or(0), 0);
        assert_eq!(completed.latest_event_id.unwrap_or(0), 42);
    }

    #[test]
    fn completion_cursor_round_trips_atomically() {
        let path = std::env::temp_dir().join(format!(
            "windie-notifier-cursor-{}-{}",
            std::process::id(),
            CURSOR_TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        assert_eq!(load_cursor_at(&path).unwrap(), None);
        save_cursor_at(&path, 42).unwrap();
        assert_eq!(load_cursor_at(&path).unwrap(), Some(42));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn completion_notification_uses_the_projected_final_text_and_session() {
        let notification = completion_notification(
            r#"{"session_id":"session-123","message":{"content":"Final answer\nwith details"}}"#,
        )
        .unwrap();

        assert_eq!(notification.session_id.as_str(), "session-123");
        assert_eq!(notification.body, "Final answer with details");
    }

    #[test]
    fn completion_notification_body_rejects_missing_or_empty_text() {
        assert!(completion_notification(r#"{}"#).is_none());
        assert!(
            completion_notification(
                r#"{"session_id":"session-123","message":{"content":" \n\t "}}"#
            )
            .is_none()
        );
    }

    #[test]
    fn notification_preview_truncates_at_a_unicode_boundary() {
        let content = "🙂".repeat(MAX_NOTIFICATION_BODY_CHARS + 1);
        let preview = notification_preview(&content).unwrap();

        assert_eq!(preview.chars().count(), MAX_NOTIFICATION_BODY_CHARS + 1);
        assert!(preview.ends_with('…'));
    }
}
