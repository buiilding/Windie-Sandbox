//! Replayable Server-Sent Events for account-scoped hosted changes.

use std::convert::Infallible;

use axum::response::sse::Event;
use futures_util::Stream;
use tokio::{
    select,
    time::{Duration, interval, sleep},
};

use super::account::HostedAccount;
use super::conversation::HostedConversationOperations;
use super::store::HostedStore;
use crate::session::{SessionEventHub, SessionId, SessionSubscriptionError};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SESSION_EVENT_RECOVERY_INTERVAL: Duration = Duration::from_secs(5);

/// Starts cross-instance delivery for durable hosted session events.
///
/// PostgreSQL sends the notification only after its transaction commits. The
/// payload is a database event ID, never account or model content; this task
/// reloads the durable record before publishing it to local SSE subscribers.
pub(crate) fn start_session_event_listener(store: HostedStore, live_events: SessionEventHub) {
    tokio::spawn(async move {
        loop {
            let mut listener = match store.session_event_listener().await {
                Ok(listener) => listener,
                Err(error) => {
                    eprintln!("hosted session-event listener failed to start: {error}");
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            loop {
                let notification = match listener.recv().await {
                    Ok(notification) => notification,
                    Err(error) => {
                        eprintln!("hosted session-event listener disconnected: {error}");
                        break;
                    }
                };
                let Ok(event_id) = notification.payload().parse::<i64>() else {
                    eprintln!("hosted session-event notification had an invalid cursor");
                    continue;
                };
                match store.session_event_by_id(event_id).await {
                    Ok(Some(record)) => live_events.publish(record),
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("hosted session-event notification reload failed: {error}");
                    }
                }
            }

            sleep(Duration::from_secs(1)).await;
        }
    });
}

/// Streams saved events after a durable cursor and then polls the same durable
/// log for new rows. Polling is deliberate: it also observes writes from a
/// second hosted-server process, whereas an in-process broadcast would not.
pub(crate) fn account_events(
    conversations: HostedConversationOperations,
    account: HostedAccount,
    after: i64,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut cursor = after;
        loop {
            match conversations.events_after(&account, cursor).await {
                Ok(events) if events.is_empty() => sleep(EVENT_POLL_INTERVAL).await,
                Ok(events) => {
                    for event in events {
                        cursor = event.id;
                        let data = serde_json::to_string(&event).unwrap_or_else(|error| {
                            serde_json::json!({"error": format!("failed to serialize hosted event: {error}")}).to_string()
                        });
                        yield Ok(Event::default().id(event.id.to_string()).event("change").data(data));
                    }
                }
                Err(error) => {
                    eprintln!("hosted event storage poll failed: {error}");
                    yield Ok(Event::default().event("error").data(
                        serde_json::json!({"error": "hosted event storage temporarily unavailable"}).to_string(),
                    ));
                    sleep(EVENT_POLL_INTERVAL).await;
                }
            }
        }
    }
}

/// Streams one account-owned session's durable history then immediate local
/// delivery. PostgreSQL notifications wake other hosted-server processes, and
/// a slow durable fallback repairs a missed notification or lagged receiver.
pub(crate) fn session_events(
    store: HostedStore,
    account: HostedAccount,
    session_id: SessionId,
    after: i64,
    live_events: SessionEventHub,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut cursor = after;
        // Subscribe before querying durable replay so no committed event can
        // land in the handoff gap.
        let mut subscription = live_events.subscribe(&session_id);
        let mut recovery = interval(SESSION_EVENT_RECOVERY_INTERVAL);
        loop {
            match store.session_events_after(&account, &session_id, cursor).await {
                Ok(events) => {
                    for record in events {
                        if record.id <= cursor {
                            continue;
                        }
                        cursor = record.id;
                        let event_name = record.event.event_name();
                        let data = serde_json::to_string(&record).unwrap_or_else(|error| {
                            serde_json::json!({"error": format!("failed to serialize hosted session event: {error}")}).to_string()
                        });
                        yield Ok(Event::default().id(record.id.to_string()).event(event_name).data(data));
                    }
                }
                Err(error) => {
                    eprintln!("hosted session event storage poll failed: {error}");
                    yield Ok(Event::default().event("error").data(
                        serde_json::json!({"error": "hosted session event storage temporarily unavailable"}).to_string(),
                    ));
                    sleep(SESSION_EVENT_RECOVERY_INTERVAL).await;
                    continue;
                }
            }

            loop {
                select! {
                    result = subscription.recv() => match result {
                        Ok(record) if record.id > cursor => {
                            cursor = record.id;
                            let event_name = record.event.event_name();
                            let data = serde_json::to_string(&record).unwrap_or_else(|error| {
                                serde_json::json!({"error": format!("failed to serialize hosted session event: {error}")}).to_string()
                            });
                            yield Ok(Event::default().id(record.id.to_string()).event(event_name).data(data));
                        }
                        Ok(_) => {}
                        Err(SessionSubscriptionError::Lagged) => break,
                        Err(SessionSubscriptionError::Closed) => {
                            subscription = live_events.subscribe(&session_id);
                            break;
                        }
                    },
                    _ = recovery.tick() => break,
                }
            }
        }
    }
}
