//! Process-local delivery of already committed session events.
//!
//! Durable storage is the source of truth for session activity. This hub only
//! gives connected clients a low-latency path to the same `SessionEventRecord`
//! after a backend has committed it. Reconnect and lag recovery always replay
//! durable records using their event IDs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use super::{SessionEventRecord, SessionId};

const SESSION_EVENT_CHANNEL_CAPACITY: usize = 256;

/// A process-local fan-out hub keyed by durable session ID.
#[derive(Clone, Default)]
pub struct SessionEventHub {
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<SessionEventRecord>>>>,
}

impl SessionEventHub {
    /// Starts a subscription before a caller replays durable records.
    ///
    /// Creating the channel here is intentional: an event committed while the
    /// caller is loading replay rows remains available to this receiver.
    pub fn subscribe(&self, session_id: &SessionId) -> SessionSubscription {
        let sender = self.sender_for(session_id);
        SessionSubscription {
            receiver: sender.subscribe(),
        }
    }

    /// Fans out one event that has already committed to durable storage.
    ///
    /// Events without active subscribers are deliberately dropped from this
    /// in-memory path. Their durable rows remain available to later replay.
    pub fn publish(&self, record: SessionEventRecord) {
        let sender = self
            .channels
            .lock()
            .expect("session event hub lock poisoned")
            .get(record.session_id.as_str())
            .cloned();
        if let Some(sender) = sender {
            let _ = sender.send(record);
        }
    }

    /// Ends the in-memory stream after a terminal session event.
    ///
    /// Existing receivers drain the terminal record before observing closure;
    /// later clients create a fresh subscription and recover history from the
    /// durable event log.
    pub fn close(&self, session_id: &SessionId) {
        self.channels
            .lock()
            .expect("session event hub lock poisoned")
            .remove(session_id.as_str());
    }

    fn sender_for(&self, session_id: &SessionId) -> broadcast::Sender<SessionEventRecord> {
        self.channels
            .lock()
            .expect("session event hub lock poisoned")
            .entry(session_id.as_str().to_string())
            .or_insert_with(|| broadcast::channel(SESSION_EVENT_CHANNEL_CAPACITY).0)
            .clone()
    }
}

/// Live subscription to events from one session.
pub struct SessionSubscription {
    receiver: broadcast::Receiver<SessionEventRecord>,
}

impl SessionSubscription {
    /// Waits for the next live event or reports that durable replay is needed.
    pub async fn recv(&mut self) -> Result<SessionEventRecord, SessionSubscriptionError> {
        match self.receiver.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Lagged(_)) => Err(SessionSubscriptionError::Lagged),
            Err(broadcast::error::RecvError::Closed) => Err(SessionSubscriptionError::Closed),
        }
    }
}

/// Reason a live subscriber must stop waiting for in-memory delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSubscriptionError {
    /// The bounded live channel dropped records; replay after the durable cursor.
    Lagged,
    /// The session's current live channel ended.
    Closed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionEvent;

    fn record(id: i64, session_id: &str) -> SessionEventRecord {
        SessionEventRecord {
            id,
            session_id: SessionId::new(session_id),
            event: SessionEvent::AssistantDelta {
                text: "hello".to_string(),
            },
            created_at: 1,
        }
    }

    #[tokio::test]
    async fn delivers_only_the_matching_session_record() {
        let hub = SessionEventHub::default();
        let first = SessionId::new("first");
        let mut subscription = hub.subscribe(&first);

        hub.publish(record(1, "second"));
        hub.publish(record(2, "first"));

        let received = subscription.recv().await.unwrap();
        assert_eq!(received.id, 2);
        assert_eq!(received.session_id, first);
    }

    #[tokio::test]
    async fn closes_after_a_terminal_delivery() {
        let hub = SessionEventHub::default();
        let session_id = SessionId::new("session");
        let mut subscription = hub.subscribe(&session_id);

        hub.publish(record(1, "session"));
        hub.close(&session_id);

        assert_eq!(subscription.recv().await.unwrap().id, 1);
        assert_eq!(
            subscription.recv().await.unwrap_err(),
            SessionSubscriptionError::Closed
        );
    }
}
