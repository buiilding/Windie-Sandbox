//! Typed CLI command data.

use super::*;

/// Parsed startup action for one `windie` process.
///
/// This is the CLI boundary's typed contract. Downstream code should match on
/// this enum instead of inspecting raw argv strings.
pub enum Command {
    /// Start the localhost developer API server.
    Api,
    /// Open the local developer inspector with the current API token.
    Inspector,
    /// Attach one provider tool to a conversation.
    AttachTool {
        conversation_id: ConversationId,
        provider_id: ToolProviderId,
        tool_name: ProviderToolName,
    },
    /// Insert one message into a conversation without model inference.
    InsertMessage {
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        role: Role,
        parts: Vec<InsertPart>,
    },
    /// Insert one root-scoped tool schema.
    InsertToolSchema {
        conversation_id: ConversationId,
        tool_schema: ToolSchema,
    },
    /// Print full read-only runtime state as JSON for developer inspection.
    Inspect {
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model: Option<ModelName>,
    },
    /// List provider tools that can be attached to conversations.
    Tools {
        provider_id: Option<ToolProviderId>,
    },
    /// Run one benchmark mode. Conversation mode carries the target
    /// conversation ID; live mode does not.
    Bench {
        mode: BenchmarkMode,
        conversation_id: Option<ConversationId>,
        options: BenchmarkOptions,
    },
    /// Compare the current local benchmark run with one stored baseline.
    CompareBaseline {
        options: BenchmarkOptions,
    },
    /// Replace one stored benchmark baseline with the current local run.
    UpdateBaseline {
        options: BenchmarkOptions,
    },
    /// Set, list, remove, or locate Windie's provider-key environment values.
    Env(EnvCommand),
    /// Install or verify one approved Windie dependency.
    Install {
        target: String,
    },
    /// Copy a conversation from the beginning through one checkpoint message.
    Fork {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    GatewayStart,
    GatewayStop,
    Help,
    Invalid,
    List {
        json: bool,
    },
    /// List models reported by the running Bifrost gateway.
    Models,
    New,
    Noop,
    SessionStart {
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model: Option<ModelName>,
    },
    SessionList {
        conversation_id: Option<ConversationId>,
    },
    SessionStatus {
        session_id: SessionId,
    },
    SessionEvents {
        session_id: SessionId,
    },
    SessionApprovals {
        session_id: SessionId,
    },
    SessionApprove {
        session_id: SessionId,
        tool_call_id: ToolCallId,
    },
    SessionDeny {
        session_id: SessionId,
        tool_call_id: ToolCallId,
    },
    SessionStop {
        session_id: SessionId,
    },
    RemoveConversation(ConversationId),
    RemoveMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    RemoveSystemPrompt(ConversationId),
    RemoveToolSchema {
        conversation_id: ConversationId,
        name: ToolSchemaName,
    },
    /// Detach one provider-backed tool schema from a conversation.
    DetachTool {
        conversation_id: ConversationId,
        schema_name: ToolSchemaName,
    },
    Show(ConversationId),
    Status,
    SetSystemPrompt {
        conversation_id: ConversationId,
        text: String,
    },
    /// Persist the conversation model used by future queries.
    SetModel {
        conversation_id: ConversationId,
        model: ModelName,
    },
    Truncate {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    Tree(ConversationId),
    UpdateMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
        text: String,
    },
    UpdateToolSchema {
        conversation_id: ConversationId,
        current_name: ToolSchemaName,
        tool_schema: ToolSchema,
    },
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One ordered input part from `windie insert`.
pub enum InsertPart {
    Text(String),
    Image(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One provider-key environment command.
pub enum EnvCommand {
    Set(Vec<(String, String)>),
    List,
    Unset(Vec<String>),
    Path,
}
