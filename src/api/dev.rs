//! Development-only API signals for exercising local presentation components.
//!
//! These routes deliberately do not enter the durable session runtime. They
//! let a developer verify that a local presentation component can receive a
//! notification-shaped signal without creating a conversation, session, or
//! replay event that could be mistaken for real assistant work.

use super::*;

/// Small, typed signal emitted only by the notifier development
/// endpoint. It is intentionally separate from `SessionEvent`: a test must
/// never claim that an assistant session completed when no session ran.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum NotifierTestNotification {
    /// Exercises the notification shown after an assistant response finishes.
    AssistantCompleted,
}

impl NotifierTestNotification {
    /// Returns the stable SSE event name understood by the native notifier.
    const fn event_name(self) -> &'static str {
        match self {
            Self::AssistantCompleted => "notifier.assistant_completed",
        }
    }
}

#[derive(Debug, Serialize)]
/// Delivery details for one development notification probe.
pub(super) struct NotifierTestNotificationResponse {
    /// Number of connected notifier SSE receivers that accepted the signal.
    notifier_receivers: usize,
}

/// Announces a volatile assistant-completed test signal to connected notifier
/// clients. A missing notifier receiver is valid: this endpoint is a manual test
/// aid and does not retain or retry signals.
pub(super) async fn report_assistant_completed_for_notifier(
    State(state): State<ApiState>,
) -> (StatusCode, Json<NotifierTestNotificationResponse>) {
    let notifier_receivers = announce_test_assistant_completion(&state.notifier_test_notifications);
    (
        StatusCode::ACCEPTED,
        Json(NotifierTestNotificationResponse { notifier_receivers }),
    )
}

/// Publishes the manual probe without making delivery a runtime guarantee.
fn announce_test_assistant_completion(
    sender: &tokio::sync::broadcast::Sender<NotifierTestNotification>,
) -> usize {
    sender
        .send(NotifierTestNotification::AssistantCompleted)
        .unwrap_or_default()
}

/// Streams development-only notifier signals to local notifier processes.
///
/// The stream has no persistence or replay cursor by design. Production
/// notifications will instead consume the existing durable session-event
/// stream, whose cursor is the source of truth for real completion facts.
pub(super) async fn notifier_test_notifications(
    State(state): State<ApiState>,
) -> Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>> {
    let receiver = state.notifier_test_notifications.subscribe();
    let stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(notification) => {
                    let data = serde_json::to_string(&notification).unwrap_or_else(|error| {
                        serde_json::json!({
                            "error": format!("failed to serialize notifier test notification: {error}")
                        })
                        .to_string()
                    });
                    let event = Event::default().event(notification.event_name()).data(data);
                    return Some((Ok(event), receiver));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // This signal is intentionally best-effort. Resume with
                    // the next message rather than terminating the notifier link.
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_completed_test_signal_has_a_namespaced_event_name() {
        assert_eq!(
            NotifierTestNotification::AssistantCompleted.event_name(),
            "notifier.assistant_completed"
        );
    }

    #[tokio::test]
    async fn assistant_completed_test_signal_is_delivered_to_live_receivers() {
        let (sender, mut receiver) = tokio::sync::broadcast::channel(1);

        assert_eq!(announce_test_assistant_completion(&sender), 1);

        assert!(matches!(
            receiver.recv().await,
            Ok(NotifierTestNotification::AssistantCompleted)
        ));
    }
}
