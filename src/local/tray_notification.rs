//! Native notification presentation and test observation.
//!
//! This component presents completed-assistant notifications and owns only the
//! development notification probe. Production notifications name a durable
//! session and, when the macOS notifier is running from its app bundle, can open
//! that session's canonical hosted Inspector URL. The probe reconnects to the
//! local API's volatile test SSE stream. Neither path changes session state.

use anyhow::{Context, Result, anyhow};
use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[cfg(target_os = "macos")]
use std::sync::OnceLock;

const RECONNECT_DELAY: Duration = Duration::from_millis(250);

/// Hosted Inspector origin used by native session-notification actions.
const INSPECTOR_ORIGIN: &str = "https://app.windieos.com";

/// Stable identifier for an explicit notification action on every platform.
const OPEN_SESSION_ACTION: &str = "windie.open-session";

/// Finite lifetime for an actionable notification response listener.
#[cfg(target_os = "macos")]
const NOTIFICATION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Records whether the installed macOS notifier received notification permission.
#[cfg(target_os = "macos")]
static NATIVE_NOTIFICATIONS_ENABLED: OnceLock<bool> = OnceLock::new();

/// Starts the development notification observer on a dedicated native thread.
///
/// The notification component runs independently from the API, so its blocking
/// SSE observer stays on a dedicated thread.
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
/// notifier exits. The connection stays open for the lifetime of the notifier, so a
/// volatile test signal cannot disappear in a reconnect gap.
fn observe_test_notifications(stopping: &AtomicBool) -> anyhow::Result<()> {
    let reader =
        crate::local::session_event_observer::open_local_sse_stream("/api/dev/notifications")?;

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
                eprintln!("windie notifier: received assistant-completed test notification");
                if let Err(error) = show_development_completion() {
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
/// This intentionally does not use a blocking HTTP client. Raw bounded
/// loopback I/O keeps the observer independent from Tokio runtime lifetime
/// rules.
/// Returns whether an SSE event is the explicit test completion signal.
fn is_assistant_completed_event(event_name: Option<&str>) -> bool {
    event_name == Some("notifier.assistant_completed")
}

/// Requests macOS notification permission for the installed notifier app.
///
/// A development notifier is an unbundled Cargo process, so it continues through
/// the AppleScript fallback without requesting permission for a non-existent
/// application identity. An installed `Windie Notifier.app` has its own identity,
/// which macOS uses both for permission and notification click callbacks.
#[cfg(target_os = "macos")]
pub(crate) fn initialize_native_notifications() {
    let enabled = match mac_usernotifications::check_bundle() {
        Err(_) => false,
        Ok(()) => match mac_usernotifications::blocking::request_auth() {
            Ok(true) => {
                eprintln!("windie notifier: macOS notifications are enabled");
                true
            }
            Ok(false) => {
                eprintln!(
                    "windie notifier: macOS notifications were denied; using the development notification fallback"
                );
                false
            }
            Err(error) => {
                eprintln!(
                    "windie notifier: could not initialize macOS notifications; using the development notification fallback: {error}"
                );
                false
            }
        },
    };
    let _ = NATIVE_NOTIFICATIONS_ENABLED.set(enabled);
}

/// Presents the development-only notification without a session navigation
/// target. Keeping this separate prevents a test probe from opening a made-up
/// Inspector location.
pub(crate) fn show_development_completion() -> Result<()> {
    show_notification("Assistant finished", None)
}

/// Presents one final assistant response and associates its click action with
/// the durable session that produced it.
pub(crate) fn show_assistant_completed(
    content: &str,
    session_id: &crate::session::SessionId,
) -> Result<()> {
    show_notification(content, Some(session_id))
}

/// Routes a notification to the platform-specific presentation layer.
#[cfg(target_os = "macos")]
fn show_notification(content: &str, session_id: Option<&crate::session::SessionId>) -> Result<()> {
    if let Some(session_id) = session_id
        && native_notifications_enabled()
    {
        return show_actionable_macos_notification(content, session_url(session_id));
    }

    show_apple_script_notification(content)
}

/// Returns whether the production notifier can use the native callback path.
#[cfg(target_os = "macos")]
fn native_notifications_enabled() -> bool {
    *NATIVE_NOTIFICATIONS_ENABLED.get_or_init(|| false)
}

/// Delivers a bundled-app notification before the durable observer advances its
/// cursor, then waits for a body click or explicit `Open session` action on a
/// dedicated worker. The notifier's run loop remains on macOS's main run loop,
/// which delivers the response callback; this worker only waits and opens the
/// already-constructed URL.
#[cfg(target_os = "macos")]
fn show_actionable_macos_notification(content: &str, session_url: String) -> Result<()> {
    use mac_usernotifications::{Action, Notification};

    let response_handle = Notification::new()
        .title("Windie")
        .message(content)
        .action(Action::button(OPEN_SESSION_ACTION, "Open session"))
        .timeout(NOTIFICATION_RESPONSE_TIMEOUT)
        .send_blocking()
        .context("macOS rejected the Windie notification")?;

    thread::Builder::new()
        .name("windie-notification-response".to_string())
        .spawn(move || {
            let response = match mac_usernotifications::block_on_current(response_handle.response())
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) | Err(error) => {
                    eprintln!("windie notifier: native notification interaction failed: {error}");
                    return;
                }
            };
            if (response.is_default_action() || response.action_identifier == OPEN_SESSION_ACTION)
                && let Err(error) = open_session_url(&session_url)
            {
                eprintln!("windie notifier: failed to open completed session: {error:#}");
            }
        })
        .context("failed to start the Windie notification worker")?;
    Ok(())
}

/// Opens an approved, session-scoped Inspector URL through macOS's browser
/// routing instead of invoking a shell.
#[cfg(target_os = "macos")]
fn open_session_url(url: &str) -> Result<()> {
    use std::process::Command;

    let status = Command::new("open")
        .arg(url)
        .status()
        .context("failed to ask macOS to open the Windie session")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("macOS open exited with {status}"))
    }
}

/// Presents an unbundled development notification through AppleScript.
#[cfg(target_os = "macos")]
fn show_apple_script_notification(content: &str) -> Result<()> {
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

/// Builds the hosted Inspector URL for one durable session.
///
/// Session IDs are generated UUIDs today, but percent-encoding the path
/// segment keeps this boundary safe if their representation evolves.
fn session_url(session_id: &crate::session::SessionId) -> String {
    format!(
        "{INSPECTOR_ORIGIN}/sessions/{}",
        percent_encode_path_segment(session_id.as_str())
    )
}

/// Encodes one URL path segment without treating any session text as URL
/// structure.
fn percent_encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

/// Escapes a normalized notification body for one AppleScript string literal.
#[cfg(target_os = "macos")]
fn apple_script_string_literal(content: &str) -> String {
    format!("\"{}\"", content.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "windows")]
fn show_notification(content: &str, session_id: Option<&crate::session::SessionId>) -> Result<()> {
    use notify_rust::{Notification, NotificationResponse};

    let mut notification = Notification::new();
    // A Win32 toast requires a Start Menu shortcut with a registered AUMID.
    // Until Windie has a Windows installer that owns that registration,
    // notify-rust's supported PowerShell identity keeps delivery functional;
    // this long-lived notifier process still receives click callbacks itself.
    notification.summary("Windie").body(content);
    let session_url = session_id.map(session_url);
    if session_url.is_some() {
        notification.action(OPEN_SESSION_ACTION, "Open session");
    }
    let response_handle = notification
        .show()
        .context("Windows rejected the Windie notification")?;
    if let Some(session_url) = session_url {
        thread::Builder::new()
            .name("windie-notification-response".to_string())
            .spawn(move || {
                if let Err(error) = response_handle.wait_for_response(|response| {
                    if response.is_default_action()
                        || matches!(response, NotificationResponse::Action(action) if action == OPEN_SESSION_ACTION)
                    {
                        if let Err(error) = open_session_url(&session_url) {
                            eprintln!("windie notifier: failed to open completed session: {error:#}");
                        }
                    }
                }) {
                    eprintln!("windie notifier: notification interaction failed: {error}");
                }
            })
            .context("failed to start the Windie notification worker")?;
    }
    Ok(())
}

/// Delivers a Freedesktop notification through the active desktop notification
/// service. A session-bearing notification keeps an interaction listener so a
/// body click or explicit action opens the hosted Inspector session.
#[cfg(all(unix, not(target_os = "macos")))]
fn show_notification(content: &str, session_id: Option<&crate::session::SessionId>) -> Result<()> {
    use notify_rust::{Notification, NotificationResponse};

    let mut notification = Notification::new();
    notification
        .appname("Windie")
        .summary("Windie")
        .body(content);
    let session_url = session_id.map(session_url);
    if session_url.is_some() {
        notification.action(OPEN_SESSION_ACTION, "Open session");
    }
    let response_handle = notification
        .show()
        .context("Linux notification service rejected the Windie notification")?;
    if let Some(session_url) = session_url {
        thread::Builder::new()
            .name("windie-notification-response".to_string())
            .spawn(move || {
                if let Err(error) = response_handle.wait_for_response(|response| {
                    if response.is_default_action()
                        || matches!(response, NotificationResponse::Action(action) if action == OPEN_SESSION_ACTION)
                    {
                        if let Err(error) = open_session_url(&session_url) {
                            eprintln!("windie notifier: failed to open completed session: {error:#}");
                        }
                    }
                }) {
                    eprintln!("windie notifier: notification interaction failed: {error}");
                }
            })
            .context("failed to start the Windie notification worker")?;
    }
    Ok(())
}

/// Opens a known hosted Inspector URL through the Windows shell association.
#[cfg(target_os = "windows")]
fn open_session_url(url: &str) -> Result<()> {
    let status = std::process::Command::new("explorer.exe")
        .arg(url)
        .status()
        .context("failed to ask Windows to open the Windie session")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Windows explorer exited with {status}"))
    }
}

/// Opens a known hosted Inspector URL through the desktop's URL handler.
#[cfg(all(unix, not(target_os = "macos")))]
fn open_session_url(url: &str) -> Result<()> {
    let status = std::process::Command::new("xdg-open")
        .arg(url)
        .status()
        .context("failed to ask Linux to open the Windie session")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("xdg-open exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn completion_event_name_matches_the_api_contract() {
        assert!(super::is_assistant_completed_event(Some(
            "notifier.assistant_completed"
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

    #[test]
    fn session_url_uses_one_escaped_session_path_segment() {
        let session = crate::session::SessionId::new("session / with spaces");

        assert_eq!(
            super::session_url(&session),
            "https://app.windieos.com/sessions/session%20%2F%20with%20spaces"
        );
    }
}
