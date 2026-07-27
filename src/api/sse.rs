//! Server-sent event helpers for streaming session events.

use std::path::Path;

use serde::Serialize;

use crate::conversation::{Message, MessagePart};
use crate::session::SessionEvent;

use super::*;

pub(super) struct SessionSseState {
    pub(super) replay: VecDeque<SessionEventRecord>,
    pub(super) subscription: Option<SessionSubscription>,
    pub(super) store_path: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct SessionEventMessage {
    id: String,
    parent_message_id: Option<String>,
    role: crate::conversation::Role,
    content: String,
    parts: Vec<SessionEventMessagePart>,
    metadata: Option<crate::conversation::MessageMetadata>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SessionEventMessagePart {
    Text {
        text: String,
    },
    Image {
        asset_id: String,
        mime_type: String,
        byte_count: usize,
    },
}

impl SessionEventMessage {
    fn from_message(message: Message) -> Option<Self> {
        Some(Self {
            id: message.id?.as_str().to_string(),
            parent_message_id: message.parent_message_id.map(|id| id.as_str().to_string()),
            role: message.role,
            content: message.content,
            parts: message
                .parts
                .into_iter()
                .map(|part| match part {
                    MessagePart::Text(text) => SessionEventMessagePart::Text { text },
                    MessagePart::Image(image) => SessionEventMessagePart::Image {
                        asset_id: image.asset_id.as_str().to_string(),
                        mime_type: image.mime_type,
                        byte_count: image.bytes.len(),
                    },
                })
                .collect(),
            metadata: message.metadata,
        })
    }
}

fn event_message_id(event: &SessionEvent) -> Option<&str> {
    match event {
        SessionEvent::InputStarted { message_id, .. }
        | SessionEvent::AssistantMessageSaved { message_id }
        | SessionEvent::ToolResultSaved { message_id } => Some(message_id),
        SessionEvent::Completed { message_id } => message_id.as_deref(),
        _ => None,
    }
}

fn includes_session_snapshot(event: &SessionEvent) -> bool {
    matches!(
        event,
        SessionEvent::InputQueued { .. }
            | SessionEvent::InputStarted { .. }
            | SessionEvent::AssistantMessageSaved { .. }
            | SessionEvent::ToolResultSaved { .. }
            | SessionEvent::WaitingForApproval
            | SessionEvent::Completed { .. }
            | SessionEvent::Failed { .. }
            | SessionEvent::Cancelled
    )
}

fn open_event_store(store_path: Option<&Path>) -> Result<Store> {
    match store_path {
        Some(path) => Store::open_at(path),
        None => Store::open(),
    }
}

/// Serializes one event and, for state-changing events, hydrates the durable
/// session and message snapshots that clients need to render without issuing
/// a follow-up conversation reload.
pub(super) fn session_event_data(store_path: Option<&Path>, record: &SessionEventRecord) -> String {
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

        if includes_session_snapshot(&record.event) {
            if let Ok(store) = open_event_store(store_path) {
                if let Ok(session) = store.load_session(&record.session_id) {
                    if let Ok(snapshot) = response_with_queue(&store, session) {
                        if let Ok(snapshot) = serde_json::to_value(snapshot) {
                            object.insert("session".to_string(), snapshot);
                        }
                    }
                }

                if let Some(message_id) = event_message_id(&record.event) {
                    let session = store.load_session(&record.session_id).ok();
                    let message = session
                        .as_ref()
                        .map(|session| {
                            store
                                .load_message(&session.conversation_id, &MessageId::new(message_id))
                        })
                        .transpose()
                        .ok()
                        .flatten()
                        .and_then(SessionEventMessage::from_message);
                    if let Some(message) =
                        message.and_then(|message| serde_json::to_value(message).ok())
                    {
                        object.insert("message".to_string(), message);
                    }
                }
            }
        }
    }

    value.to_string()
}
