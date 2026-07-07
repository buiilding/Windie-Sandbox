//! Tests for runtime flow coordination.

use anyhow::{Result, anyhow};
use std::sync::Mutex;

use super::*;
use crate::conversation::{
    Message, MessageMetadata, ToolCall, ToolCallId, ToolSchema, ToolSchemaName,
};
use crate::llm::{AssistantResponse, FinishReason};
use crate::tool::ToolApprovalMode;

struct NoopOutput;

impl RuntimeOutput for NoopOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[ToolCall]) {}
}

struct FailingLlm;

impl RuntimeLlm for FailingLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Err(anyhow!("llm failed"))
    }
}

struct ReplyLlm {
    reply: String,
}

struct CapturingLlm {
    messages: Mutex<Vec<Message>>,
    tools: Mutex<Vec<ToolSchema>>,
}

impl CapturingLlm {
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            tools: Mutex::new(Vec::new()),
        }
    }
}

impl RuntimeLlm for CapturingLlm {
    async fn stream<F>(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        *self.messages.lock().unwrap() = messages.to_vec();
        *self.tools.lock().unwrap() = tools.to_vec();
        handle_delta("captured")?;

        Ok(AssistantResponse {
            content: "captured".to_string(),
            metadata: MessageMetadata::default(),
            finish_reason: Some(FinishReason::Stop),
        })
    }
}

impl ReplyLlm {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
        }
    }
}

impl RuntimeLlm for ReplyLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        handle_delta(&self.reply)?;

        Ok(AssistantResponse {
            content: self.reply.clone(),
            metadata: MessageMetadata::default(),
            finish_reason: Some(FinishReason::Stop),
        })
    }
}

struct ToolCallLlm;

impl RuntimeLlm for ToolCallLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Ok(AssistantResponse {
            content: String::new(),
            metadata: MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"ls"}"#,
                )],
                ..Default::default()
            },
            finish_reason: Some(FinishReason::ToolCalls),
        })
    }
}

struct UnknownToolCallLlm;

impl RuntimeLlm for UnknownToolCallLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Ok(AssistantResponse {
            content: String::new(),
            metadata: MessageMetadata {
                tool_calls: vec![ToolCall::function("call_unknown", "unknown_tool", "{}")],
                ..Default::default()
            },
            finish_reason: Some(FinishReason::ToolCalls),
        })
    }
}

struct UnknownThenShellToolCallLlm;

impl RuntimeLlm for UnknownThenShellToolCallLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        Ok(AssistantResponse {
            content: String::new(),
            metadata: MessageMetadata {
                tool_calls: vec![
                    ToolCall::function("call_unknown", "unknown_tool", "{}"),
                    ToolCall::function("call_shell", "run_shell", r#"{"command":"printf ok"}"#),
                ],
                ..Default::default()
            },
            finish_reason: Some(FinishReason::ToolCalls),
        })
    }
}

struct ToolThenReplyLlm {
    calls: Mutex<usize>,
    second_turn_messages: Mutex<Vec<Message>>,
}

impl ToolThenReplyLlm {
    fn new() -> Self {
        Self {
            calls: Mutex::new(0),
            second_turn_messages: Mutex::new(Vec::new()),
        }
    }
}

impl RuntimeLlm for ToolThenReplyLlm {
    async fn stream<F>(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: FnMut(&str) -> Result<()>,
    {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;

        if *calls == 1 {
            return Ok(AssistantResponse {
                content: String::new(),
                metadata: MessageMetadata {
                    tool_calls: vec![ToolCall::function(
                        "call_123",
                        "run_shell",
                        r#"{"command":"printf windie-shell"}"#,
                    )],
                    ..Default::default()
                },
                finish_reason: Some(FinishReason::ToolCalls),
            });
        }

        *self.second_turn_messages.lock().unwrap() = messages.to_vec();
        handle_delta("done")?;

        Ok(AssistantResponse {
            content: "done".to_string(),
            metadata: MessageMetadata::default(),
            finish_reason: Some(FinishReason::Stop),
        })
    }
}

#[tokio::test]
async fn query_conversation_saves_assistant_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let assistant_message = query_conversation_once(
        &NoopOutput,
        &ReplyLlm::new("hello back"),
        &mut store,
        &conversation_id,
    )
    .await
    .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(assistant_message.role, Role::Assistant);
    assert_eq!(assistant_message.content, "hello back");
    assert_eq!(
        assistant_message.parent_message_id.as_deref(),
        Some(user_id.as_str())
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].content, "hello back");
    assert_eq!(
        messages[1].parent_message_id.as_deref(),
        messages[0].id.as_deref()
    );
    assert_eq!(
        store
            .active_message_id(&conversation_id)
            .unwrap()
            .as_deref(),
        messages[1].id.as_deref()
    );
}

#[tokio::test]
async fn query_conversation_uses_active_path() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let active_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "active",
            None,
        )
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "inactive",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &active_id)
        .unwrap();
    let llm = CapturingLlm::new();

    query_conversation_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();

    let captured = llm.messages.lock().unwrap();

    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].content, "root");
    assert_eq!(captured[1].content, "active");
}

#[tokio::test]
async fn query_conversation_passes_tool_schemas_to_llm() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };
    store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    let llm = CapturingLlm::new();

    query_conversation_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();

    assert_eq!(*llm.tools.lock().unwrap(), vec![tool_schema]);
}

#[tokio::test]
async fn query_approve_query_composes_shell_tool_flow() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_run_shell_schema(&mut store, &conversation_id);
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();
    let llm = ToolThenReplyLlm::new();

    let tool_call_message =
        query_conversation_once(&NoopOutput, &llm, &mut store, &conversation_id)
            .await
            .unwrap();
    let result = approve_tool_call(&mut store, &conversation_id, &ToolCallId::new("call_123"))
        .await
        .unwrap();
    let assistant_message =
        query_conversation_once(&NoopOutput, &llm, &mut store, &conversation_id)
            .await
            .unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();
    let second_turn_messages = llm.second_turn_messages.lock().unwrap();

    assert_eq!(tool_call_message.role, Role::Assistant);
    assert_eq!(
        tool_call_message
            .metadata
            .as_ref()
            .map(|metadata| metadata.tool_calls.len()),
        Some(1)
    );
    assert!(result.success);
    assert_eq!(assistant_message.role, Role::Assistant);
    assert_eq!(assistant_message.content, "done");
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("windie-shell"));
    assert_eq!(
        messages[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(|id| id.as_str()),
        Some("call_123")
    );
    assert_eq!(messages[3].role, Role::Assistant);
    assert_eq!(second_turn_messages.len(), 3);
    assert_eq!(second_turn_messages[2].role, Role::Tool);
    assert!(second_turn_messages[2].content.contains("windie-shell"));
}

#[tokio::test]
async fn auto_approval_executes_tool_and_queries_again() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_run_shell_schema(&mut store, &conversation_id);
    store
        .set_tool_approval_mode(&conversation_id, ToolApprovalMode::AutoApproveAttached)
        .unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();
    let llm = ToolThenReplyLlm::new();
    let registry = ToolProviderRegistry::new();

    let assistant_message = query_conversation_resolving_automatic_tools(
        &NoopOutput,
        &llm,
        &mut store,
        &conversation_id,
        &registry,
    )
    .await
    .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();
    let second_turn_messages = llm.second_turn_messages.lock().unwrap();

    assert_eq!(assistant_message.role, Role::Assistant);
    assert_eq!(assistant_message.content, "done");
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("windie-shell"));
    assert_eq!(messages[3].role, Role::Assistant);
    assert!(approvals.is_empty());
    assert_eq!(second_turn_messages.len(), 3);
    assert_eq!(second_turn_messages[2].role, Role::Tool);
}

#[tokio::test]
async fn query_conversation_once_saves_tool_calls_without_executing() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_run_shell_schema(&mut store, &conversation_id);
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();

    query_conversation_once(&NoopOutput, &ToolCallLlm, &mut store, &conversation_id)
        .await
        .unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();
    let metadata = messages[1].metadata.as_ref().unwrap();

    assert_eq!(messages.len(), 2);
    assert!(messages[1].content.is_empty());
    assert_eq!(metadata.tool_calls.len(), 1);
    assert_eq!(metadata.tool_calls[0].id.as_str(), "call_123");
    assert_eq!(metadata.tool_calls[0].name(), "run_shell");
    assert_eq!(metadata.tool_calls[0].arguments(), r#"{"command":"ls"}"#);
}

#[tokio::test]
async fn query_conversation_once_auto_stores_policy_denied_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_tool_schema(&mut store, &conversation_id, "unknown_tool");
    store
        .insert_message(&conversation_id, None, Role::User, "use a tool", None)
        .unwrap();

    query_conversation_once(
        &NoopOutput,
        &UnknownToolCallLlm,
        &mut store,
        &conversation_id,
    )
    .await
    .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("unknown tool: unknown_tool"));
    assert_eq!(
        messages[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(ToolCallId::as_str),
        Some("call_unknown")
    );
    assert!(approvals.is_empty());
    validate_query_availability(&store, &conversation_id).unwrap();
}

#[tokio::test]
async fn detached_shell_tool_call_is_auto_denied() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();

    query_conversation_once(&NoopOutput, &ToolCallLlm, &mut store, &conversation_id)
        .await
        .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(
        messages[2]
            .content
            .contains("Tool is not attached: run_shell")
    );
    assert!(approvals.is_empty());
    validate_query_availability(&store, &conversation_id).unwrap();
}

#[test]
fn removed_shell_schema_makes_existing_pending_call_policy_denied() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_run_shell_schema(&mut store, &conversation_id);
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"ls"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();
    store
        .remove_tool_schema(&conversation_id, &ToolSchemaName::new("run_shell"))
        .unwrap();

    prepare_query_turn(&mut store, &conversation_id).unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages[2].role, Role::Tool);
    assert!(
        messages[2]
            .content
            .contains("Tool is not attached: run_shell")
    );
    assert!(approvals.is_empty());
}

#[tokio::test]
async fn policy_denied_tool_results_stop_before_tool_calls_requiring_approval() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_tool_schema(&mut store, &conversation_id, "unknown_tool");
    attach_run_shell_schema(&mut store, &conversation_id);
    store
        .insert_message(&conversation_id, None, Role::User, "use tools", None)
        .unwrap();

    query_conversation_once(
        &NoopOutput,
        &UnknownThenShellToolCallLlm,
        &mut store,
        &conversation_id,
    )
    .await
    .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("unknown tool: unknown_tool"));
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_shell");
}

#[tokio::test]
async fn prepare_query_turn_resolves_existing_policy_denied_tool_call_before_query() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_tool_schema(&mut store, &conversation_id, "unknown_tool");
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "use a tool", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function("call_unknown", "unknown_tool", "{}")],
                ..Default::default()
            }),
        )
        .unwrap();
    let llm = CapturingLlm::new();

    prepare_query_turn(&mut store, &conversation_id).unwrap();
    query_conversation_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();
    let captured = llm.messages.lock().unwrap();

    assert_eq!(captured.len(), 3);
    assert_eq!(captured[2].role, Role::Tool);
    assert!(captured[2].content.contains("unknown tool: unknown_tool"));
    assert_eq!(
        captured[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(ToolCallId::as_str),
        Some("call_unknown")
    );
}

#[tokio::test]
async fn pending_tool_approvals_lists_pending_shell_calls() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_run_shell_schema(&mut store, &conversation_id);
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "run a command", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"printf approved"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_123");
    assert_eq!(approvals[0].tool_call.name(), "run_shell");
    assert_eq!(approvals[0].reason, "shell tool requires approval");
}

#[tokio::test]
async fn pending_tool_approvals_ignores_inactive_branch_tool_calls() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "run a command", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_inactive",
                    "run_shell",
                    r#"{"command":"printf inactive"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::User,
            "use this branch instead",
            None,
        )
        .unwrap();

    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();
    let error = approve_tool_call(
        &mut store,
        &conversation_id,
        &ToolCallId::new("call_inactive"),
    )
    .await
    .unwrap_err();

    assert!(approvals.is_empty());
    assert!(
        error
            .to_string()
            .contains("pending tool call does not exist")
    );
}

#[tokio::test]
async fn approve_tool_call_executes_shell_and_stores_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    attach_run_shell_schema(&mut store, &conversation_id);
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "run a command", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"printf approved"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let result = approve_tool_call(&mut store, &conversation_id, &ToolCallId::new("call_123"))
        .await
        .unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert!(result.success);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("approved"));
    assert_eq!(
        messages[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(|id| id.as_str()),
        Some("call_123")
    );
    assert!(approvals.is_empty());
}

#[test]
fn deny_tool_call_stores_rejected_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "run a command", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"printf denied"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let result =
        deny_tool_call(&mut store, &conversation_id, &ToolCallId::new("call_123")).unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(!result.success);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("tool call rejected by user"));
}

#[tokio::test]
async fn multi_tool_approvals_resolve_in_metadata_order() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let (_assistant_id, _first_call, _second_call) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();

    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_1");
}

#[tokio::test]
async fn multi_tool_approvals_store_results_as_linear_chain() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let (assistant_id, first_call_id, second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    approve_tool_call(&mut store, &conversation_id, &first_call_id)
        .await
        .unwrap();
    let approvals = pending_tool_approvals(&store, &conversation_id).unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_2");

    approve_tool_call(&mut store, &conversation_id, &second_call_id)
        .await
        .unwrap();
    let llm = CapturingLlm::new();
    let final_message = query_conversation_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let captured = llm.messages.lock().unwrap();

    assert_eq!(messages.len(), 5);
    assert_eq!(messages[1].id.as_ref(), Some(&assistant_id));
    assert_eq!(messages[2].role, Role::Tool);
    assert_eq!(
        messages[2].parent_message_id.as_deref(),
        Some(assistant_id.as_str())
    );
    assert_eq!(
        messages[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(ToolCallId::as_str),
        Some("call_1")
    );
    assert_eq!(messages[3].role, Role::Tool);
    assert_eq!(
        messages[3].parent_message_id.as_deref(),
        messages[2].id.as_deref()
    );
    assert_eq!(
        messages[3]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(ToolCallId::as_str),
        Some("call_2")
    );
    assert_eq!(
        final_message.parent_message_id.as_deref(),
        messages[3].id.as_deref()
    );
    assert_eq!(
        captured
            .iter()
            .map(|message| message.role)
            .collect::<Vec<_>>(),
        vec![Role::User, Role::Assistant, Role::Tool, Role::Tool]
    );
}

#[tokio::test]
async fn approving_later_tool_call_before_previous_call_rejects() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let (_assistant_id, _first_call_id, second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    let error = approve_tool_call(&mut store, &conversation_id, &second_call_id)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "tool call must be resolved after previous tool call: call_1"
    );
}

#[tokio::test]
async fn query_rejects_until_all_tool_calls_have_results() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let (_assistant_id, first_call_id, _second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    let first_error =
        query_conversation_once(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
            .await
            .unwrap_err();
    approve_tool_call(&mut store, &conversation_id, &first_call_id)
        .await
        .unwrap();
    let second_error =
        query_conversation_once(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
            .await
            .unwrap_err();

    assert_eq!(
        first_error.to_string(),
        "tool call requires result before query: call_1"
    );
    assert_eq!(
        second_error.to_string(),
        "tool call requires result before query: call_2"
    );
}

#[test]
fn denying_multi_tool_call_uses_linear_chain_parent() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let (_assistant_id, first_call_id, second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    deny_tool_call(&mut store, &conversation_id, &first_call_id).unwrap();
    deny_tool_call(&mut store, &conversation_id, &second_call_id).unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();

    assert_eq!(messages[2].role, Role::Tool);
    assert_eq!(messages[3].role, Role::Tool);
    assert_eq!(
        messages[3].parent_message_id.as_deref(),
        messages[2].id.as_deref()
    );
    assert!(messages[3].content.contains("tool call rejected by user"));
}

#[tokio::test]
async fn query_conversation_reports_llm_failure() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    let error = query_conversation_once(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "llm failed");
}

fn insert_multi_tool_call_assistant(
    store: &mut Store,
    conversation_id: &ConversationId,
) -> (MessageId, ToolCallId, ToolCallId) {
    attach_run_shell_schema(store, conversation_id);
    let user_id = store
        .insert_message(conversation_id, None, Role::User, "run commands", None)
        .unwrap();
    let first_call_id = ToolCallId::new("call_1");
    let second_call_id = ToolCallId::new("call_2");
    let assistant_id = store
        .insert_message(
            conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![
                    ToolCall::function("call_1", "run_shell", r#"{"command":"printf first"}"#),
                    ToolCall::function("call_2", "run_shell", r#"{"command":"printf second"}"#),
                ],
                ..Default::default()
            }),
        )
        .unwrap();

    (assistant_id, first_call_id, second_call_id)
}

fn attach_run_shell_schema(store: &mut Store, conversation_id: &ConversationId) {
    let registry = crate::tool_provider::ToolProviderRegistry::new();
    let attached_tool = registry
        .find_tool(
            &crate::tool::ToolProviderId::new("windie"),
            &crate::tool::ProviderToolName::new("run_shell"),
        )
        .unwrap()
        .unwrap()
        .attached_tool();

    store
        .insert_attached_tool(conversation_id, &attached_tool)
        .unwrap();
}

fn attach_tool_schema(store: &mut Store, conversation_id: &ConversationId, name: &str) {
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new(name),
        description: format!("{name} test tool"),
        parameters: serde_json::json!({"type":"object"}),
    };

    store
        .insert_tool_schema(conversation_id, &tool_schema)
        .unwrap();
}
