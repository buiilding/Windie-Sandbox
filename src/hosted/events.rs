//! Replayable Server-Sent Events for account-scoped hosted changes.

use std::convert::Infallible;

use axum::response::sse::Event;
use futures_util::Stream;
use tokio::time::{Duration, sleep};

use super::account::HostedAccount;
use super::conversation::HostedConversationOperations;
use super::store::HostedStore;
use crate::session::SessionId;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(250);

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

/// Streams one account-owned session's durable event history and then polls for
/// later rows. The database cursor makes reconnect recovery independent of a
/// specific `windie-server` process.
pub(crate) fn session_events(
    store: HostedStore,
    account: HostedAccount,
    session_id: SessionId,
    after: i64,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut cursor = after;
        loop {
            match store.session_events_after(&account, &session_id, cursor).await {
                Ok(events) if events.is_empty() => sleep(EVENT_POLL_INTERVAL).await,
                Ok(events) => {
                    for record in events {
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
                    sleep(EVENT_POLL_INTERVAL).await;
                }
            }
        }
    }
}
