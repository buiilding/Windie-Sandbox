//! Native tray notification test observer.
//!
//! This component owns only the development notification probe: it reconnects
//! to the local API's volatile test SSE stream and turns the explicit
//! assistant-completed signal into a native operating-system notification. It
//! neither starts runtime components nor reads or writes session state.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const RECONNECT_DELAY: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

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
    let reader = open_test_notification_stream()?;

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
                show_assistant_completed();
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
fn open_test_notification_stream() -> anyhow::Result<BufReader<TcpStream>> {
    let address = crate::config::api_address();
    let socket = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not resolve Windie API address {address}"))?;
    let mut stream = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT)?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    stream.write_all(
        format!(
            "GET /api/dev/tray-notifications HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
        )
        .as_bytes(),
    )?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    if !status_line.starts_with("HTTP/1.1 200") {
        return Err(anyhow::anyhow!(
            "Windie API notification stream returned {}",
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

/// Returns whether an SSE event is the explicit test completion signal.
fn is_assistant_completed_event(event_name: Option<&str>) -> bool {
    event_name == Some("tray.assistant_completed")
}

/// Presents the one notification used by the manual completion probe.
#[cfg(target_os = "macos")]
fn show_assistant_completed() {
    use std::process::Command;

    match Command::new("osascript")
        .args([
            "-e",
            "display notification \"Assistant finished\" with title \"Windie\"",
        ])
        .status()
    {
        Ok(status) if !status.success() => {
            eprintln!("failed to show Windie notification: osascript exited with {status}");
        }
        Ok(_) => {}
        Err(error) => eprintln!("failed to show Windie notification: {error}"),
    }
}

/// Keeps the tray buildable on Windows while native Windows toast delivery is
/// intentionally deferred until the durable notification observer is added.
#[cfg(target_os = "windows")]
fn show_assistant_completed() {
    eprintln!("Windie tray received an assistant-completed test notification");
}

/// The tray itself is unavailable on these targets; keep the library's local
/// observer component portable for test and documentation builds.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn show_assistant_completed() {
    eprintln!("Windie tray received an assistant-completed test notification");
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
}
