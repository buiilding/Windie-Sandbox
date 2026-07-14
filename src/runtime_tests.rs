//! Tests for runtime flow coordination.

use anyhow::{Result, anyhow};
use std::fs;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::conversation::{
    Message, MessageMetadata, ToolCall, ToolCallId, ToolSchema, ToolSchemaName,
};
use crate::llm::{
    AssistantResponse, FinishReason, LlmStreamEvent, PromptCacheRequest, ReasoningRequest,
};
use crate::mcp::McpCommand;
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolApprovalMode, ToolExecutionResult, ToolPermission,
    ToolProviderId, ToolProviderKind, ToolProviderRef,
};
use crate::tool_provider::ToolProviderRegistry;

const TEST_PROVIDER_ID: &str = "desktop-commander";
const TEST_PROVIDER_PREFIX: &str = "desktop_commander";
const TEST_PROVIDER_DISPLAY_NAME: &str = "Desktop Commander";
const TEST_PROVIDER_TOOL_NAME: &str = "read_file";
const TEST_TOOL_SCHEMA_NAME: &str = "desktop_commander__read_file";
const TEST_TOOL_RESULT: &str = "test-mcp-output";

static TEMP_MCP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct NoopOutput;

impl RuntimeOutput for NoopOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[ToolCall]) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedRuntimeEvent {
    AssistantMessageSaved(MessageId),
    ToolResultSaved(MessageId),
}

struct RecordingRuntimeEvents {
    events: Mutex<Vec<RecordedRuntimeEvent>>,
}

impl RecordingRuntimeEvents {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl RuntimeEventSink for RecordingRuntimeEvents {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedRuntimeEvent::AssistantMessageSaved(
                message_id.clone(),
            ));
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedRuntimeEvent::ToolResultSaved(message_id.clone()));
    }
}

struct FailingLlm;

impl RuntimeLlm for FailingLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
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
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        *self.messages.lock().unwrap() = messages.to_vec();
        *self.tools.lock().unwrap() = tools.to_vec();
        handle_delta(LlmStreamEvent::AssistantDelta("captured"))?;

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
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        handle_delta(LlmStreamEvent::AssistantDelta(&self.reply))?;

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
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        Ok(AssistantResponse {
            content: String::new(),
            metadata: MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    TEST_TOOL_SCHEMA_NAME,
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
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
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

struct UnknownThenProviderToolCallLlm;

impl RuntimeLlm for UnknownThenProviderToolCallLlm {
    async fn stream<F>(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        _handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        Ok(AssistantResponse {
            content: String::new(),
            metadata: MessageMetadata {
                tool_calls: vec![
                    ToolCall::function("call_unknown", "unknown_tool", "{}"),
                    ToolCall::function(
                        "call_provider",
                        TEST_TOOL_SCHEMA_NAME,
                        r#"{"command":"printf ok"}"#,
                    ),
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
        _reasoning: Option<&ReasoningRequest>,
        _prompt_cache: Option<&PromptCacheRequest>,
        mut handle_delta: F,
    ) -> Result<AssistantResponse>
    where
        F: for<'a> FnMut(LlmStreamEvent<'a>) -> Result<()>,
    {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;

        if *calls == 1 {
            return Ok(AssistantResponse {
                content: String::new(),
                metadata: MessageMetadata {
                    tool_calls: vec![ToolCall::function(
                        "call_123",
                        TEST_TOOL_SCHEMA_NAME,
                        r#"{"command":"printf windie-shell"}"#,
                    )],
                    ..Default::default()
                },
                finish_reason: Some(FinishReason::ToolCalls),
            });
        }

        *self.second_turn_messages.lock().unwrap() = messages.to_vec();
        handle_delta(LlmStreamEvent::AssistantDelta("done"))?;

        Ok(AssistantResponse {
            content: "done".to_string(),
            metadata: MessageMetadata::default(),
            finish_reason: Some(FinishReason::Stop),
        })
    }
}

fn active_head(store: &Store, conversation_id: &ConversationId) -> Option<MessageId> {
    store.active_message_id(conversation_id).unwrap()
}

fn path_to_head(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: &MessageId,
) -> Vec<Message> {
    store
        .load_path_to_message(conversation_id, head_message_id)
        .unwrap()
}

fn prepare_active_head_turn(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    let registry = ToolProviderRegistry::new();
    let events = NoopRuntimeEventSink;
    let mut head_message_id = active_head(store, conversation_id);

    prepare_head_turn(
        store,
        conversation_id,
        &mut head_message_id,
        &registry,
        &events,
    )?;
    if let Some(head_message_id) = head_message_id {
        store.set_active_message(conversation_id, &head_message_id)?;
    }

    Ok(())
}

fn pending_active_head_approvals(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<Vec<crate::tool::ToolApprovalRequest>> {
    let registry = ToolProviderRegistry::new();
    let head_message_id = active_head(store, conversation_id);

    pending_approvals_at_head(
        store,
        RuntimeInput {
            conversation_id,
            head_message_id: head_message_id.as_ref(),
            tools: &registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
    )
}

fn validate_active_head_availability(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<()> {
    let head_message_id = active_head(store, conversation_id);
    let registry = ToolProviderRegistry::new();

    pending_approvals_at_head(
        store,
        RuntimeInput {
            conversation_id,
            head_message_id: head_message_id.as_ref(),
            tools: &registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
    )?;
    let messages = match head_message_id.as_ref() {
        Some(message_id) => store.load_path_to_message(conversation_id, message_id)?,
        None => Vec::new(),
    };
    let Some(execution) = active_tool_execution(&messages) else {
        return Ok(());
    };
    let Some(tool_call) = execution.next_pending_tool_call() else {
        return Ok(());
    };

    Err(error::invalid_request(format!(
        "tool call requires result before query: {}",
        tool_call.id
    )))
}

async fn run_active_head_once<O, L>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    let registry = ToolProviderRegistry::new();
    let events = NoopRuntimeEventSink;

    run_active_head_once_with_registry_and_events(
        output,
        llm,
        store,
        conversation_id,
        &registry,
        &events,
        RuntimeModelRequest::new(None, None),
    )
    .await
}

async fn run_active_head_once_with_registry_and_events<O, L, E>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &E,
    model_request: RuntimeModelRequest<'_>,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
    E: RuntimeEventSink,
{
    let head_message_id = active_head(store, conversation_id);

    let message = advance_turn(
        output,
        llm,
        store,
        RuntimeInput {
            conversation_id,
            head_message_id: head_message_id.as_ref(),
            tools: registry,
            model_request,
        },
        events,
    )
    .await?;
    if let Some(message_id) = store
        .load_messages(conversation_id)?
        .last()
        .and_then(|message| message.id.as_ref())
        .cloned()
        .or_else(|| message.id.clone())
    {
        store.set_active_message(conversation_id, &message_id)?;
    }

    Ok(message)
}

async fn run_active_head_until_blocked<O, L>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    reasoning: Option<&ReasoningRequest>,
    prompt_cache: Option<&PromptCacheRequest>,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    let events = NoopRuntimeEventSink;

    run_active_head_until_blocked_with_events(
        output,
        llm,
        store,
        conversation_id,
        registry,
        &events,
        RuntimeModelRequest::new(reasoning, prompt_cache),
    )
    .await
}

async fn run_active_head_until_blocked_with_events<O, L, E>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &E,
    model_request: RuntimeModelRequest<'_>,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
    E: RuntimeEventSink,
{
    let head_message_id = active_head(store, conversation_id);
    let outcome = advance_until_blocked(
        output,
        llm,
        store,
        RuntimeInput {
            conversation_id,
            head_message_id: head_message_id.as_ref(),
            tools: registry,
            model_request,
        },
        events,
    )
    .await?;
    let head_message_id = match outcome {
        RuntimeOutcome::Completed {
            head_message_id: Some(head_message_id),
        }
        | RuntimeOutcome::WaitingForApproval { head_message_id } => head_message_id,
        RuntimeOutcome::Completed {
            head_message_id: None,
        } => return Err(anyhow!("run did not create a head message")),
    };
    let messages = path_to_head(store, conversation_id, &head_message_id);
    store.set_active_message(conversation_id, &head_message_id)?;

    messages
        .into_iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .ok_or_else(|| anyhow!("run did not create an assistant message"))
}

async fn approve_active_head_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    let registry = ToolProviderRegistry::new();

    approve_active_head_tool_call_with_registry(store, conversation_id, tool_call_id, &registry)
        .await
}

async fn approve_active_head_tool_call_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    let head_message_id = active_head(store, conversation_id);
    let pending = load_pending_tool_call_at_head(
        store,
        conversation_id,
        head_message_id.as_ref(),
        tool_call_id,
    )?;
    let execution = prepare_pending_tool_execution(
        store,
        conversation_id,
        head_message_id.as_ref(),
        &pending,
        registry,
    )?;
    let result = match execution {
        PendingToolExecution::Finished(result) => result,
        PendingToolExecution::Execute(attached_tool) => {
            execute_pending_tool_call(&pending, &attached_tool, registry).await?
        }
    };
    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    store.set_active_message(conversation_id, &message_id)?;

    Ok(result)
}

fn deny_active_head_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    let head_message_id = active_head(store, conversation_id);
    let pending = load_pending_tool_call_at_head(
        store,
        conversation_id,
        head_message_id.as_ref(),
        tool_call_id,
    )?;
    let result = deny_pending_tool_call(&pending);
    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    store.set_active_message(conversation_id, &message_id)?;

    Ok(result)
}

#[tokio::test]
async fn run_head_saves_assistant_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let assistant_message = run_active_head_once(
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
async fn two_explicit_head_sessions_create_sibling_assistant_messages() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "branch here", None)
        .unwrap();
    let registry = ToolProviderRegistry::new();
    let events = NoopRuntimeEventSink;

    let first_outcome = advance_until_blocked(
        &NoopOutput,
        &ReplyLlm::new("first branch"),
        &mut store,
        RuntimeInput {
            conversation_id: &conversation_id,
            head_message_id: Some(&user_id),
            tools: &registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
        &events,
    )
    .await
    .unwrap();
    let second_outcome = advance_until_blocked(
        &NoopOutput,
        &ReplyLlm::new("second branch"),
        &mut store,
        RuntimeInput {
            conversation_id: &conversation_id,
            head_message_id: Some(&user_id),
            tools: &registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
        &events,
    )
    .await
    .unwrap();

    let first_id = match first_outcome {
        RuntimeOutcome::Completed {
            head_message_id: Some(message_id),
        } => message_id,
        _ => panic!("first explicit-head execution did not complete"),
    };
    let second_id = match second_outcome {
        RuntimeOutcome::Completed {
            head_message_id: Some(message_id),
        } => message_id,
        _ => panic!("second explicit-head execution did not complete"),
    };
    let first_path = path_to_head(&store, &conversation_id, &first_id);
    let second_path = path_to_head(&store, &conversation_id, &second_id);

    assert_ne!(first_id, second_id);
    assert_eq!(first_path.len(), 2);
    assert_eq!(second_path.len(), 2);
    assert_eq!(first_path[0].id.as_ref(), Some(&user_id));
    assert_eq!(second_path[0].id.as_ref(), Some(&user_id));
    assert_eq!(first_path[1].content, "first branch");
    assert_eq!(second_path[1].content, "second branch");
    assert_eq!(
        first_path[1].parent_message_id.as_deref(),
        Some(user_id.as_str())
    );
    assert_eq!(
        second_path[1].parent_message_id.as_deref(),
        Some(user_id.as_str())
    );
}

#[tokio::test]
async fn run_head_uses_active_path() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
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

    run_active_head_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();

    let captured = llm.messages.lock().unwrap();

    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].content, "root");
    assert_eq!(captured[1].content, "active");
}

#[tokio::test]
async fn run_head_passes_tool_schemas_to_llm() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
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

    run_active_head_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();

    assert_eq!(*llm.tools.lock().unwrap(), vec![tool_schema]);
}

#[tokio::test]
async fn explicit_run_head_uses_that_branch_prompt_and_tools() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let shared_id = store
        .insert_message(&conversation_id, Some(&root_id), Role::User, "shared", None)
        .unwrap();
    store
        .set_active_message(&conversation_id, &shared_id)
        .unwrap();
    store
        .set_system_prompt(&conversation_id, "shared prompt")
        .unwrap();

    let branch_tool = ToolSchema {
        name: ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };
    let branch_id = store
        .insert_message(
            &conversation_id,
            Some(&shared_id),
            Role::User,
            "branch",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &branch_id)
        .unwrap();
    store
        .set_system_prompt(&conversation_id, "branch prompt")
        .unwrap();
    store
        .insert_tool_schema(&conversation_id, &branch_tool)
        .unwrap();
    let sibling_id = store
        .insert_message(
            &conversation_id,
            Some(&shared_id),
            Role::User,
            "sibling",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &sibling_id)
        .unwrap();
    let llm = CapturingLlm::new();
    let events = NoopRuntimeEventSink;
    let registry = ToolProviderRegistry::new();

    advance_turn(
        &NoopOutput,
        &llm,
        &mut store,
        RuntimeInput {
            conversation_id: &conversation_id,
            head_message_id: Some(&branch_id),
            tools: &registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
        &events,
    )
    .await
    .unwrap();

    let captured_messages = llm.messages.lock().unwrap();

    assert_eq!(captured_messages[0].role, Role::System);
    assert_eq!(captured_messages[0].content, "branch prompt");
    assert_eq!(*llm.tools.lock().unwrap(), vec![branch_tool]);
}

#[tokio::test]
async fn session_approve_run_composes_provider_tool_flow() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();
    let llm = ToolThenReplyLlm::new();
    let registry = test_mcp_registry();

    let tool_call_message = run_active_head_once(&NoopOutput, &llm, &mut store, &conversation_id)
        .await
        .unwrap();
    let result = approve_active_head_tool_call_with_registry(
        &mut store,
        &conversation_id,
        &ToolCallId::new("call_123"),
        &registry,
    )
    .await
    .unwrap();
    let assistant_message = run_active_head_once(&NoopOutput, &llm, &mut store, &conversation_id)
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
    assert!(messages[2].content.contains(TEST_TOOL_RESULT));
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
    assert!(second_turn_messages[2].content.contains(TEST_TOOL_RESULT));
}

#[tokio::test]
async fn auto_approval_executes_tool_and_queries_again() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
    store
        .set_tool_approval_mode(&conversation_id, ToolApprovalMode::AutoApproveAttached)
        .unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();
    let llm = ToolThenReplyLlm::new();
    let registry = test_mcp_registry();

    let assistant_message = run_active_head_until_blocked(
        &NoopOutput,
        &llm,
        &mut store,
        &conversation_id,
        &registry,
        None,
        None,
    )
    .await
    .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();
    let second_turn_messages = llm.second_turn_messages.lock().unwrap();

    assert_eq!(assistant_message.role, Role::Assistant);
    assert_eq!(assistant_message.content, "done");
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains(TEST_TOOL_RESULT));
    assert_eq!(messages[3].role, Role::Assistant);
    assert!(approvals.is_empty());
    assert_eq!(second_turn_messages.len(), 3);
    assert_eq!(second_turn_messages[2].role, Role::Tool);
}

#[tokio::test]
async fn auto_approval_emits_persisted_session_events() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
    store
        .set_tool_approval_mode(&conversation_id, ToolApprovalMode::AutoApproveAttached)
        .unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();
    let llm = ToolThenReplyLlm::new();
    let registry = test_mcp_registry();
    let events = RecordingRuntimeEvents::new();

    run_active_head_until_blocked_with_events(
        &NoopOutput,
        &llm,
        &mut store,
        &conversation_id,
        &registry,
        &events,
        RuntimeModelRequest::new(None, None),
    )
    .await
    .unwrap();

    let messages = store.load_active_path(&conversation_id).unwrap();
    let recorded = events.events.lock().unwrap().clone();

    assert_eq!(
        recorded,
        vec![
            RecordedRuntimeEvent::AssistantMessageSaved(messages[1].id.clone().unwrap()),
            RecordedRuntimeEvent::ToolResultSaved(messages[2].id.clone().unwrap()),
            RecordedRuntimeEvent::AssistantMessageSaved(messages[3].id.clone().unwrap()),
        ]
    );
}

#[tokio::test]
async fn run_active_head_once_saves_tool_calls_without_executing() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();

    run_active_head_once(&NoopOutput, &ToolCallLlm, &mut store, &conversation_id)
        .await
        .unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();
    let metadata = messages[1].metadata.as_ref().unwrap();

    assert_eq!(messages.len(), 2);
    assert!(messages[1].content.is_empty());
    assert_eq!(metadata.tool_calls.len(), 1);
    assert_eq!(metadata.tool_calls[0].id.as_str(), "call_123");
    assert_eq!(metadata.tool_calls[0].name(), TEST_TOOL_SCHEMA_NAME);
    assert_eq!(metadata.tool_calls[0].arguments(), r#"{"command":"ls"}"#);
}

#[tokio::test]
async fn run_active_head_once_auto_stores_policy_denied_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_tool_schema(&mut store, &conversation_id, "unknown_tool");
    store
        .insert_message(&conversation_id, None, Role::User, "use a tool", None)
        .unwrap();

    run_active_head_once(
        &NoopOutput,
        &UnknownToolCallLlm,
        &mut store,
        &conversation_id,
    )
    .await
    .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

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
    validate_active_head_availability(&store, &conversation_id).unwrap();
}

#[tokio::test]
async fn detached_tool_call_is_auto_denied() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "list files", None)
        .unwrap();

    run_active_head_once(&NoopOutput, &ToolCallLlm, &mut store, &conversation_id)
        .await
        .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(
        messages[2]
            .content
            .contains("Tool is not attached: desktop_commander__read_file")
    );
    assert!(approvals.is_empty());
    validate_active_head_availability(&store, &conversation_id).unwrap();
}

#[test]
fn removed_tool_schema_makes_existing_pending_call_policy_denied() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
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
                    TEST_TOOL_SCHEMA_NAME,
                    r#"{"command":"ls"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();
    store
        .remove_tool_schema(
            &conversation_id,
            &ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
        )
        .unwrap();

    prepare_active_head_turn(&mut store, &conversation_id).unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages[2].role, Role::Tool);
    assert!(
        messages[2]
            .content
            .contains("Tool is not attached: desktop_commander__read_file")
    );
    assert!(approvals.is_empty());
}

#[tokio::test]
async fn policy_denied_tool_results_stop_before_tool_calls_requiring_approval() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_tool_schema(&mut store, &conversation_id, "unknown_tool");
    attach_test_mcp_tool(&mut store, &conversation_id);
    store
        .insert_message(&conversation_id, None, Role::User, "use tools", None)
        .unwrap();

    run_active_head_once(
        &NoopOutput,
        &UnknownThenProviderToolCallLlm,
        &mut store,
        &conversation_id,
    )
    .await
    .unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("unknown tool: unknown_tool"));
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_provider");
}

#[tokio::test]
async fn prepare_active_head_turn_resolves_existing_policy_denied_tool_call_before_query() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
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

    prepare_active_head_turn(&mut store, &conversation_id).unwrap();
    run_active_head_once(&NoopOutput, &llm, &mut store, &conversation_id)
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
async fn pending_active_head_approvals_lists_pending_provider_calls() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
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
                    TEST_TOOL_SCHEMA_NAME,
                    r#"{"command":"printf approved"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_123");
    assert_eq!(approvals[0].tool_call.name(), TEST_TOOL_SCHEMA_NAME);
    assert_eq!(approvals[0].reason, "tool requires approval");
}

#[tokio::test]
async fn pending_active_head_approvals_ignores_inactive_branch_tool_calls() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
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
                    TEST_TOOL_SCHEMA_NAME,
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

    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();
    let error = approve_active_head_tool_call(
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
async fn approve_active_head_tool_call_executes_provider_and_stores_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    attach_test_mcp_tool(&mut store, &conversation_id);
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
                    TEST_TOOL_SCHEMA_NAME,
                    r#"{"command":"printf approved"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let registry = test_mcp_registry();
    let result = approve_active_head_tool_call_with_registry(
        &mut store,
        &conversation_id,
        &ToolCallId::new("call_123"),
        &registry,
    )
    .await
    .unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

    assert!(result.success);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains(TEST_TOOL_RESULT));
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
fn deny_active_head_tool_call_stores_rejected_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
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
                    TEST_TOOL_SCHEMA_NAME,
                    r#"{"command":"printf denied"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let result =
        deny_active_head_tool_call(&mut store, &conversation_id, &ToolCallId::new("call_123"))
            .unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(!result.success);
    assert_eq!(messages[2].role, Role::Tool);
    assert!(messages[2].content.contains("tool call rejected by user"));
}

#[tokio::test]
async fn multi_tool_approvals_resolve_in_metadata_order() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let (_assistant_id, _first_call, _second_call) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();

    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_1");
}

#[tokio::test]
async fn multi_tool_approvals_store_results_as_linear_chain() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let (assistant_id, first_call_id, second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);
    let registry = test_mcp_registry();

    approve_active_head_tool_call_with_registry(
        &mut store,
        &conversation_id,
        &first_call_id,
        &registry,
    )
    .await
    .unwrap();
    let approvals = pending_active_head_approvals(&store, &conversation_id).unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool_call.id.as_str(), "call_2");

    approve_active_head_tool_call_with_registry(
        &mut store,
        &conversation_id,
        &second_call_id,
        &registry,
    )
    .await
    .unwrap();
    let llm = CapturingLlm::new();
    let final_message = run_active_head_once(&NoopOutput, &llm, &mut store, &conversation_id)
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
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let (_assistant_id, _first_call_id, second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    let error = approve_active_head_tool_call(&mut store, &conversation_id, &second_call_id)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "tool call must be resolved after previous tool call: call_1"
    );
}

#[tokio::test]
async fn run_rejects_until_all_tool_calls_have_results() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let (_assistant_id, first_call_id, _second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);
    let registry = test_mcp_registry();

    let first_error = run_active_head_once(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
        .await
        .unwrap_err();
    approve_active_head_tool_call_with_registry(
        &mut store,
        &conversation_id,
        &first_call_id,
        &registry,
    )
    .await
    .unwrap();
    let second_error = run_active_head_once(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
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
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let (_assistant_id, first_call_id, second_call_id) =
        insert_multi_tool_call_assistant(&mut store, &conversation_id);

    deny_active_head_tool_call(&mut store, &conversation_id, &first_call_id).unwrap();
    deny_active_head_tool_call(&mut store, &conversation_id, &second_call_id).unwrap();
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
async fn run_head_reports_llm_failure() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let error = run_active_head_once(&NoopOutput, &FailingLlm, &mut store, &conversation_id)
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "llm failed");
}

fn insert_multi_tool_call_assistant(
    store: &mut Store,
    conversation_id: &ConversationId,
) -> (MessageId, ToolCallId, ToolCallId) {
    attach_test_mcp_tool(store, conversation_id);
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
                    ToolCall::function(
                        "call_1",
                        TEST_TOOL_SCHEMA_NAME,
                        r#"{"command":"printf first"}"#,
                    ),
                    ToolCall::function(
                        "call_2",
                        TEST_TOOL_SCHEMA_NAME,
                        r#"{"command":"printf second"}"#,
                    ),
                ],
                ..Default::default()
            }),
        )
        .unwrap();

    (assistant_id, first_call_id, second_call_id)
}

fn attach_test_mcp_tool(store: &mut Store, conversation_id: &ConversationId) {
    store
        .insert_attached_tool(conversation_id, &test_tool_definition().attached_tool())
        .unwrap();
}

fn test_mcp_registry() -> ToolProviderRegistry {
    ToolProviderRegistry::with_test_mcp_provider(
        TEST_PROVIDER_ID,
        TEST_PROVIDER_PREFIX,
        TEST_PROVIDER_DISPLAY_NAME,
        test_mcp_command(),
        vec![test_tool_definition()],
    )
}

fn test_tool_definition() -> crate::tool::ToolDefinition {
    crate::tool::ToolDefinition {
        schema_name: ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
        display_name: "Desktop Commander read_file".to_string(),
        description: "Read a file through Desktop Commander.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new(TEST_PROVIDER_ID),
            ProviderToolName::new(TEST_PROVIDER_TOOL_NAME),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

fn test_mcp_command() -> McpCommand {
    let path = write_test_mcp_server();
    let program = Box::leak(path.into_boxed_str());

    McpCommand {
        program,
        args: &[],
        env: &[],
    }
}

fn write_test_mcp_server() -> String {
    use std::os::unix::fs::PermissionsExt;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEMP_MCP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "windie-runtime-test-mcp-{}-{nanos}-{counter}.sh",
        std::process::id()
    ));
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"windie-test-mcp","version":"0"}}}}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"{tool_name}","description":"Test tool","inputSchema":{{"type":"object"}}}}]}}}}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"content":[{{"type":"text","text":"{tool_result}"}}],"isError":false}}}}'
      ;;
  esac
done
"#,
        tool_name = TEST_PROVIDER_TOOL_NAME,
        tool_result = TEST_TOOL_RESULT
    );
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();

    path.to_string_lossy().into_owned()
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
