//! Tests for the SQLite persistence boundary.

use super::*;
use crate::conversation::{
    MessagePart, TokenUsage, ToolCall, ToolSchema, ToolSchemaName, UnsavedImagePart,
    UnsavedMessagePart,
};
use crate::tool::ToolApprovalMode;

fn unsaved_text(text: &str) -> UnsavedMessagePart {
    UnsavedMessagePart::Text(text.to_string())
}

fn unsaved_image(mime_type: &str, bytes: &[u8]) -> UnsavedMessagePart {
    UnsavedMessagePart::Image(UnsavedImagePart {
        mime_type: mime_type.to_string(),
        bytes: bytes.to_vec(),
    })
}

fn index_exists(store: &Store, index_name: &str) -> bool {
    store
        .connection
        .query_row(
            "
            SELECT EXISTS (
                SELECT 1
                FROM sqlite_master
                WHERE type = 'index'
                  AND name = ?1
            )
            ",
            [index_name],
            |row| row.get(0),
        )
        .unwrap()
}

fn message_parent<'a>(messages: &'a [Message], message_id: &MessageId) -> Option<&'a MessageId> {
    messages
        .iter()
        .find(|message| message.id.as_ref() == Some(message_id))
        .and_then(|message| message.parent_message_id.as_ref())
}

fn message_ids(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| message.id.as_ref())
        .map(ToString::to_string)
        .collect()
}

fn image_asset_count(store: &Store) -> i64 {
    store
        .connection
        .query_row("SELECT COUNT(*) FROM image_assets", [], |row| row.get(0))
        .unwrap()
}

fn insert_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &str,
    content: &str,
) -> MessageId {
    store
        .insert_tool_result_message(
            conversation_id,
            parent_message_id,
            &ToolCallId::new(tool_call_id),
            content,
        )
        .unwrap()
}

fn insert_tool_result_with_parts(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &str,
    content: &str,
    parts: &[UnsavedMessagePart],
) -> MessageId {
    store
        .insert_tool_result_message_on_branch(
            conversation_id,
            parent_message_id,
            &ToolCallId::new(tool_call_id),
            content,
            parts,
        )
        .unwrap()
}

mod compactions;
mod conversations;
mod messages;
mod schema;
mod tools;
