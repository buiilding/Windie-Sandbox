//! Typed CLI command data.

use super::*;
use crate::perf::BenchmarkOptions;

/// Parsed startup action for one `windie` process.
///
/// This is the CLI boundary's typed contract. Downstream code should match on
/// this enum instead of inspecting raw argv strings.
pub enum Command {
    /// Run a repository development workflow through the public CLI.
    Dev(DevCommand),
    /// Run a repository release workflow through the public CLI.
    Release(ReleaseCommand),
    /// Build or serve the repository's local marketplace through the public CLI.
    Marketplace(MarketplaceCommand),
    /// Run, compare, or update deterministic local benchmarks.
    Benchmark(BenchmarkCommand),
    /// Start the detached localhost developer API server.
    ApiStart,
    /// Stop the detached localhost developer API server.
    ApiStop,
    /// Print the detached API process output.
    ApiOutput,
    /// Internal foreground API server entrypoint used by `api start`.
    ApiRun,
    /// Start the detached Inspector server.
    InspectorStart,
    /// Stop the detached Inspector server.
    InspectorStop,
    /// Print the detached Inspector process output.
    InspectorOutput,
    /// Run the terminal-only first-run onboarding wizard.
    Onboard,
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
    /// Print the detached Bifrost process output.
    GatewayOutput,
    /// Run the desktop tray controller.
    Tray,
    /// Remove Windie's processes, local data, and installed binaries.
    Uninstall {
        yes: bool,
        dry_run: bool,
    },
    Help,
    Invalid,
    List {
        json: bool,
    },
    /// List models reported by the running Bifrost gateway.
    Models,
    New,
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

/// Repository development workflow selected by `windie dev`.
pub enum DevCommand {
    Up,
    Run { component: DevComponent },
    Status,
    Down,
}

/// Foreground repository component selected by `windie dev run`.
pub enum DevComponent {
    Gateway,
    Api,
    Inspector,
}

/// Repository release workflow selected by `windie release`.
pub enum ReleaseCommand {
    Build,
    Install,
    Verify,
}

/// Local marketplace workflow selected by `windie marketplace`.
pub enum MarketplaceCommand {
    Build,
    Serve,
    /// Publish archives to GitHub Releases and the catalog site to Vercel.
    Publish,
}

/// Deterministic local benchmark workflow selected by the public CLI.
pub enum BenchmarkCommand {
    Run {
        conversation_id: Option<ConversationId>,
        options: BenchmarkOptions,
    },
    CompareBaseline {
        options: BenchmarkOptions,
    },
    UpdateBaseline {
        options: BenchmarkOptions,
    },
}
