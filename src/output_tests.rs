//! Tests for terminal output formatting.

use super::*;
use crate::conversation::{ConversationId, MessageId, Role};

#[test]
fn formats_empty_conversations() {
    let lines = conversation_lines(&[]);

    assert_eq!(lines, vec!["no conversations"]);
}

#[test]
fn formats_help_lines() {
    let lines = help_lines();

    assert_eq!(lines[0], "windie");
    assert!(lines.contains(&"Usage:".to_string()));
    assert!(lines.contains(&"  windie".to_string()));
    assert!(lines.contains(&"  windie ls".to_string()));
    assert!(lines.contains(&"  windie show <conversation_id>".to_string()));
    assert!(lines.contains(
        &"  windie update <conversation_id> <message_id> --text \"new text\"".to_string()
    ));
    assert!(lines.contains(&"  windie query <conversation_id>".to_string()));
    assert!(lines.contains(&"  windie gateway start".to_string()));
    assert!(lines.contains(&"  windie gateway stop".to_string()));
    assert!(lines.contains(&"  windie bench <conversation_id>".to_string()));
    assert!(lines.contains(&"  windie bench live".to_string()));
    assert!(lines.contains(&"Options:".to_string()));
}

#[test]
fn formats_invalid_usage_lines() {
    let lines = invalid_usage_lines();

    assert_eq!(lines[0], "invalid usage");
    assert_eq!(lines[1], "");
    assert_eq!(lines[2], "windie");
    assert!(lines.contains(&"Usage:".to_string()));
}

#[test]
fn formats_conversations() {
    let conversations = vec![
        ConversationInfo {
            id: ConversationId::new("first"),
            title: None,
            message_count: 1,
        },
        ConversationInfo {
            id: ConversationId::new("second"),
            title: None,
            message_count: 2,
        },
    ];

    let lines = conversation_lines(&conversations);

    assert_eq!(
        lines,
        vec!["conversations", "first  1 message", "second  2 messages"]
    );
}

#[test]
fn formats_conversation_title() {
    let conversations = vec![ConversationInfo {
        id: ConversationId::new("chat-id"),
        title: Some("work notes".to_string()),
        message_count: 3,
    }];

    let lines = conversation_lines(&conversations);

    assert_eq!(
        lines,
        vec!["conversations", "chat-id  3 messages  work notes"]
    );
}

#[test]
fn formats_empty_messages() {
    let lines = message_lines(&[]);

    assert_eq!(lines, vec!["no messages"]);
}

#[test]
fn formats_messages_with_ids_and_roles() {
    let messages = vec![
        Message {
            id: Some(MessageId::new("user-id")),
            parent_message_id: None,
            role: Role::User,
            content: "hello".to_string(),
            metadata: None,
        },
        Message {
            id: Some(MessageId::new("assistant-id")),
            parent_message_id: Some(MessageId::new("user-id")),
            role: Role::Assistant,
            content: "hello back".to_string(),
            metadata: None,
        },
    ];

    let lines = message_lines(&messages);

    assert_eq!(
        lines,
        vec![
            "messages",
            "user  user-id  hello",
            "assistant  assistant-id  hello back"
        ]
    );
}

#[test]
fn formats_unsaved_message_id() {
    let messages = vec![Message {
        id: None,
        parent_message_id: None,
        role: Role::User,
        content: "draft".to_string(),
        metadata: None,
    }];

    let lines = message_lines(&messages);

    assert_eq!(lines, vec!["messages", "user  <unsaved>  draft"]);
}

#[test]
fn normalizes_message_preview_whitespace() {
    assert_eq!(message_preview("hello\n\n  back"), "hello back");
}

#[test]
fn truncates_long_message_preview() {
    let preview = message_preview(
        "1234567890 1234567890 1234567890 1234567890 1234567890 1234567890 1234567890 1234567890 1234567890",
    );

    assert_eq!(preview.len(), 83);
    assert!(preview.ends_with("..."));
}

#[test]
fn truncates_unicode_message_preview_without_byte_slicing() {
    let preview = message_preview(&"你".repeat(81));

    assert_eq!(preview.chars().count(), 83);
    assert!(preview.ends_with("..."));
}

#[test]
fn formats_duration_as_microseconds() {
    let duration = std::time::Duration::from_micros(42);

    assert_eq!(format_duration(duration), "42us");
}

#[test]
fn formats_duration_as_milliseconds() {
    let duration = std::time::Duration::from_millis(42);

    assert_eq!(format_duration(duration), "42ms");
}

#[test]
fn formats_duration_as_seconds() {
    let duration = std::time::Duration::from_millis(1420);

    assert_eq!(format_duration(duration), "1.42s");
}

#[test]
fn live_benchmark_mode_reports_provider_call() {
    assert!(crate::perf::BenchmarkMode::Live.may_call_provider());
    assert!(!crate::perf::BenchmarkMode::Conversation.may_call_provider());
    assert!(!crate::perf::BenchmarkMode::Local.may_call_provider());
}
