//! Terminal output boundary.
//!
//! This module owns CLI printing for assistant streams and command output.
//! Other modules should pass display data here instead of formatting terminal
//! output themselves.

use std::io::{self, Write};

use anyhow::{Context, Result};

use crate::conversation::{ConversationId, Message, MessageId};
use crate::perf::PerformanceBaseline;
use crate::store::ConversationInfo;

pub(crate) trait RuntimeOutput {
    fn start_assistant_message(&self);
    fn assistant_delta(&self, text: &str) -> Result<()>;
    fn end_assistant_message(&self);
}

pub struct TerminalOutput;

impl TerminalOutput {
    pub fn help(&self) {
        print_lines(&help_lines());
    }

    pub fn invalid_usage(&self) {
        print_lines(&invalid_usage_lines());
    }

    pub fn version(&self) {
        println!("windie {}", env!("CARGO_PKG_VERSION"));
    }

    pub fn performance_baseline(&self, baseline: &PerformanceBaseline) {
        println!("performance baseline");
        println!("mode: {}", baseline.mode.as_str());
        if baseline.mode.may_call_provider() {
            println!("warning: live benchmark sent a real provider request and may cost money");
        }
        println!("model: {}", baseline.model);
        if let Some(conversation_id) = baseline.conversation_id.as_ref() {
            println!("conversation: {conversation_id}");
        }
        if let Some(duration) = baseline.store_open {
            println!("store open: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.conversation_load {
            println!("conversation load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_build {
            println!("context build: {}", format_duration(duration));
        }
        if let Some(loaded_messages) = baseline.loaded_messages {
            println!("loaded messages: {loaded_messages}");
        }
        if let Some(duration) = baseline.gateway_ready {
            println!("gateway ready: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.first_token {
            println!("first token: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.full_response {
            println!("full response: {}", format_duration(duration));
        }
        if let Some(response_bytes) = baseline.response_bytes {
            println!("response bytes: {response_bytes}");
        }
    }

    pub fn created_conversation(&self, conversation_id: &ConversationId) {
        println!("{conversation_id}");
    }

    pub fn appended_message(&self, message_id: &MessageId) {
        println!("{message_id}");
    }

    pub fn updated_message(&self, message_id: &MessageId) {
        println!("updated message {message_id}");
    }

    pub fn removed_conversation(&self, conversation_id: &ConversationId) {
        println!("removed conversation {conversation_id}");
    }

    pub fn removed_message(&self, message_id: &MessageId) {
        println!("removed message {message_id}");
    }

    pub fn truncated_conversation(
        &self,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) {
        println!("truncated conversation {conversation_id} after message {message_id}");
    }

    pub fn forked_conversation(&self, conversation_id: &ConversationId) {
        println!("{conversation_id}");
    }

    pub fn status(&self, gateway_running: bool) {
        println!("status");
        println!(
            "gateway: {}",
            if gateway_running {
                "running"
            } else {
                "not running"
            }
        );
    }

    pub fn gateway_started(&self) {
        println!("gateway: started");
    }

    pub fn gateway_already_running(&self) {
        println!("gateway: already running");
    }

    pub fn gateway_stopped(&self) {
        println!("gateway: stopped");
    }

    pub fn gateway_not_running(&self) {
        println!("gateway: not running");
    }

    pub fn conversations(&self, conversations: &[ConversationInfo]) {
        for line in conversation_lines(conversations) {
            println!("{line}");
        }
    }

    pub fn conversation_messages(&self, messages: &[Message]) {
        for line in message_lines(messages) {
            println!("{line}");
        }
    }

    pub fn start_assistant_message(&self) {
        println!();
    }

    pub fn assistant_delta(&self, text: &str) -> Result<()> {
        print!("{text}");
        io::stdout()
            .flush()
            .context("failed to flush assistant output")
    }

    pub fn end_assistant_message(&self) {
        println!("\n");
    }
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

fn help_lines() -> Vec<String> {
    vec![
        "windie",
        "",
        "Usage:",
        "  windie",
        "  windie new",
        "  windie ls",
        "  windie show <conversation_id>",
        "  windie append <conversation_id> --role user --text \"hello\"",
        "  windie update <conversation_id> <message_id> --text \"new text\"",
        "  windie rm <conversation_id>",
        "  windie rm <conversation_id> <message_id>",
        "  windie truncate <conversation_id> <message_id>",
        "  windie fork <conversation_id> <message_id>",
        "  windie query <conversation_id>",
        "  windie query <conversation_id> --model openai/gpt-4o-mini",
        "  windie status",
        "  windie gateway start",
        "  windie gateway stop",
        "  windie bench",
        "  windie bench <conversation_id>",
        "  windie bench live",
        "",
        "Notes:",
        "  windie exits successfully without runtime action.",
        "  windie bench measures local store open only.",
        "  windie bench <conversation_id> measures conversation load and context build.",
        "  windie gateway start starts the local Bifrost gateway.",
        "  windie gateway stop stops the local Bifrost gateway.",
        "  windie query requires the local Bifrost gateway to be running.",
        "  windie bench live sends a real provider request and may cost money.",
        "",
        "Options:",
        "  -h, --help       Show help",
        "  -V, --version    Show version",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn invalid_usage_lines() -> Vec<String> {
    let mut lines = vec!["invalid usage".to_string(), String::new()];
    lines.extend(help_lines());
    lines
}

impl RuntimeOutput for TerminalOutput {
    fn start_assistant_message(&self) {
        TerminalOutput::start_assistant_message(self);
    }

    fn assistant_delta(&self, text: &str) -> Result<()> {
        TerminalOutput::assistant_delta(self, text)
    }

    fn end_assistant_message(&self) {
        TerminalOutput::end_assistant_message(self);
    }
}

fn message_count(count: i64) -> String {
    if count == 1 {
        "1 message".to_string()
    } else {
        format!("{count} messages")
    }
}

fn conversation_lines(conversations: &[ConversationInfo]) -> Vec<String> {
    if conversations.is_empty() {
        return vec!["no conversations".to_string()];
    }

    let mut lines = vec!["conversations".to_string()];

    for conversation in conversations {
        let count = message_count(conversation.message_count);

        if let Some(title) = conversation
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
        {
            lines.push(format!("{}  {count}  {title}", conversation.id));
        } else {
            lines.push(format!("{}  {count}", conversation.id));
        }
    }

    lines
}

fn message_lines(messages: &[Message]) -> Vec<String> {
    if messages.is_empty() {
        return vec!["no messages".to_string()];
    }

    let mut lines = vec!["messages".to_string()];

    for message in messages {
        let id = message
            .id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or("<unsaved>");
        lines.push(format!(
            "{}  {}  {}",
            message.role.as_str(),
            id,
            message_preview(&message.content)
        ));
    }

    lines
}

fn message_preview(content: &str) -> String {
    let preview = content.split_whitespace().collect::<Vec<_>>().join(" ");

    let truncated = preview.chars().take(80).collect::<String>();
    if truncated.len() == preview.len() {
        return preview;
    }

    format!("{truncated}...")
}

fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.2}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}us", duration.as_micros())
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
