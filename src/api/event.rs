//! Aggregate runtime-event HTTP routes.
//!
//! Session-specific streams remain the detailed client surface. This module
//! exposes one database-wide cursor so clients can observe durable activity
//! across every session, including work run by a different Windie process.

use super::*;

#[derive(Debug, Deserialize)]
/// Cursor and optional typed filter for aggregate session events.
pub(super) struct GlobalEventsQuery {
    pub(super) after: Option<i64>,
    pub(super) kind: Option<crate::session::SessionEventKind>,
}

#[derive(Debug, Serialize)]
/// Current durable cursor for an aggregate event selection.
pub(super) struct GlobalEventCursorResponse {
    latest_event_id: Option<i64>,
}

/// Returns the latest durable cursor for first-time aggregate consumers.
pub(super) async fn global_event_cursor(
    State(state): State<ApiState>,
    Query(query): Query<GlobalEventsQuery>,
) -> ApiResult<GlobalEventCursorResponse> {
    let store = open_store(&state)?;
    Ok(Json(GlobalEventCursorResponse {
        latest_event_id: store.latest_global_session_event_id(query.kind)?,
    }))
}

/// Streams durable session events from every session in global row order.
pub(super) async fn global_events(
    State(state): State<ApiState>,
    Query(query): Query<GlobalEventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    let store = open_store(&state)?;
    let latest = store.latest_global_session_event_id(query.kind)?;
    let cursor = match (query.after, latest) {
        (Some(requested), Some(latest)) if requested <= latest => requested,
        (Some(_), Some(latest)) => latest,
        (Some(requested), None) if requested <= 0 => requested,
        (Some(_), None) | (None, None) => 0,
        (None, Some(latest)) => latest,
    };
    let stream = stream::unfold(
        GlobalSseState::new(store, cursor, query.kind),
        |mut state| async move {
            let record = state.next_record().await;
            let event_name = global_event_name(&record.event);
            let sse = Event::default()
                .id(record.id.to_string())
                .event(&event_name)
                .data(global_event_data(&record));

            Some((Ok::<Event, Infallible>(sse), state))
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
