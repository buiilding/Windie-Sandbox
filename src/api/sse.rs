//! Server-sent event helpers for streaming session events.

use super::*;

pub(super) struct SessionSseState {
    pub(super) replay: VecDeque<SessionEventRecord>,
    pub(super) subscription: Option<SessionSubscription>,
}

pub(super) fn session_event_data(record: &SessionEventRecord) -> String {
    let mut value = serde_json::to_value(&record.event).unwrap_or_else(|error| {
        serde_json::json!({
            "type": "failed",
            "error": format!("failed to serialize runtime event: {error}"),
            "causes": [format!("failed to serialize runtime event: {error}")],
        })
    });
    if let Some(object) = value.as_object_mut() {
        object.insert("event_id".to_string(), serde_json::json!(record.id));
        object.insert(
            "session_id".to_string(),
            serde_json::json!(record.session_id.as_str()),
        );
        object.insert(
            "created_at".to_string(),
            serde_json::json!(record.created_at),
        );
    }

    value.to_string()
}
