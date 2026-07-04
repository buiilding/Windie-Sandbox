//! Tests for terminal output formatting.

use super::*;
use crate::conversation::{ConversationId, MessageId, Role};
use crate::perf::{
    BenchmarkMode, DurationMetric, PerformanceComparison, PerformanceComparisonRow,
    PerformanceReport, PerformanceSummary,
};

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
    assert!(lines.contains(&"  windie activate <conversation_id> <message_id>".to_string()));
    assert!(lines.contains(&"  windie show <conversation_id>".to_string()));
    assert!(lines.contains(&"  windie tree <conversation_id>".to_string()));
    assert!(
        lines.contains(
            &"  windie insert <conversation_id> --role user --text \"hello\"".to_string()
        )
    );
    assert!(lines.contains(
        &"  windie update <conversation_id> <message_id> --text \"new text\"".to_string()
    ));
    assert!(lines.contains(
        &"  windie set systemprompt <conversation_id> --text \"system prompt\"".to_string()
    ));
    assert!(lines.contains(&"  windie truncate <conversation_id> <message_id>".to_string()));
    assert!(lines.contains(&"  windie fork <conversation_id> <message_id>".to_string()));
    assert!(lines.contains(&"  windie query <conversation_id>".to_string()));
    assert!(lines.contains(&"  windie gateway start".to_string()));
    assert!(lines.contains(&"  windie gateway stop".to_string()));
    assert!(lines.contains(&"  windie bench ls".to_string()));
    assert!(lines.contains(&"  windie bench <conversation_id>".to_string()));
    assert!(lines.contains(&"  windie bench <conversation_id> --runs 100 --json".to_string()));
    assert!(lines.contains(&"  windie bench compare <baseline.json> <current.json>".to_string()));
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
            parts: Vec::new(),
            metadata: None,
        },
        Message {
            id: Some(MessageId::new("assistant-id")),
            parent_message_id: Some(MessageId::new("user-id")),
            role: Role::Assistant,
            content: "hello back".to_string(),
            parts: Vec::new(),
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
        parts: Vec::new(),
        metadata: None,
    }];

    let lines = message_lines(&messages);

    assert_eq!(lines, vec!["messages", "user  <unsaved>  draft"]);
}

#[test]
fn formats_message_tree_with_active_marker() {
    let messages = vec![
        Message {
            id: Some(MessageId::new("root-id")),
            parent_message_id: None,
            role: Role::User,
            content: "root".to_string(),
            parts: Vec::new(),
            metadata: None,
        },
        Message {
            id: Some(MessageId::new("active-id")),
            parent_message_id: Some(MessageId::new("root-id")),
            role: Role::Assistant,
            content: "active".to_string(),
            parts: Vec::new(),
            metadata: None,
        },
        Message {
            id: Some(MessageId::new("branch-id")),
            parent_message_id: Some(MessageId::new("root-id")),
            role: Role::Assistant,
            content: "branch".to_string(),
            parts: Vec::new(),
            metadata: None,
        },
    ];

    let lines = tree_lines(&messages, Some(&MessageId::new("active-id")));

    assert_eq!(
        lines,
        vec![
            "tree",
            "  user  root-id  root",
            "  * assistant  active-id  active",
            "    assistant  branch-id  branch",
        ]
    );
}

#[test]
fn normalizes_message_preview_whitespace() {
    assert_eq!(text_preview("hello\n\n  back"), "hello back");
}

#[test]
fn truncates_long_message_preview() {
    let preview = text_preview(
        "1234567890 1234567890 1234567890 1234567890 1234567890 1234567890 1234567890 1234567890 1234567890",
    );

    assert_eq!(preview.len(), 83);
    assert!(preview.ends_with("..."));
}

#[test]
fn truncates_unicode_message_preview_without_byte_slicing() {
    let preview = text_preview(&"你".repeat(81));

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
fn formats_performance_report_lines() {
    let report = PerformanceReport {
        format_version: 1,
        mode: BenchmarkMode::Conversation,
        model: "openai/gpt-4o-mini".to_string(),
        conversation_id: Some("conversation-id".to_string()),
        runs: 3,
        samples: vec![],
        summary: PerformanceSummary {
            store_open: Some(DurationMetric {
                min_us: 100,
                median_us: 200,
                p95_us: 300,
                max_us: 400,
            }),
            active_path_load: None,
            tree_load: None,
            context_build: None,
            conversation_list_load: None,
            gateway_ready: None,
            first_token: None,
            full_response: None,
        },
    };

    let lines = performance_report_lines(&report);

    assert_eq!(lines[0], "performance report");
    assert!(lines.contains(&"mode: conversation".to_string()));
    assert!(lines.contains(&"runs: 3".to_string()));
    assert!(lines.contains(&"store open:".to_string()));
    assert!(lines.contains(&"  median: 200us".to_string()));
}

#[test]
fn formats_performance_comparison_lines() {
    let comparison = PerformanceComparison {
        baseline_mode: BenchmarkMode::Conversation,
        current_mode: BenchmarkMode::Conversation,
        baseline_runs: 100,
        current_runs: 100,
        rows: vec![PerformanceComparisonRow {
            name: "context build",
            baseline_median_us: 100,
            current_median_us: 80,
            change_percent: -20.0,
        }],
    };

    let lines = performance_comparison_lines(&comparison);

    assert_eq!(lines[0], "performance comparison");
    assert!(lines.contains(&"baseline: conversation (100 runs)".to_string()));
    assert!(lines.contains(&"current: conversation (100 runs)".to_string()));
    assert!(lines.contains(&"context build: 100us -> 80us (-20.0%)".to_string()));
}

#[test]
fn live_benchmark_mode_reports_provider_call() {
    assert!(crate::perf::BenchmarkMode::Live.may_call_provider());
    assert!(!crate::perf::BenchmarkMode::Conversation.may_call_provider());
    assert!(!crate::perf::BenchmarkMode::Local.may_call_provider());
}
