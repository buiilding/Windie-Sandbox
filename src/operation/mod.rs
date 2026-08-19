//! Shared CLI/API operation layer.
//!
//! This module owns the orchestration that should be identical across clients:
//! loading inspection snapshots, inserting messages, mutating conversation
//! state, and resolving explicit tool approvals. CLI and API code translate
//! inputs into these typed operations and translate returned values into their
//! own output formats.

mod component;
mod conversation;
mod gateway;
mod input;
mod inspection;
mod message;
mod onboarding;
mod session;
mod session_approval;
mod session_cli;
mod system;
mod tool;

pub use component::*;
pub use conversation::*;
pub use gateway::*;
pub use input::{MessageInputPart, PreparedMessageInput, prepare_message_input};
pub use inspection::*;
pub use message::*;
pub use onboarding::*;
pub use session::*;
pub use session_approval::*;
pub use session_cli::*;
pub use system::*;
pub use tool::*;

#[cfg(test)]
use gateway::{SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE, conversation_prompt_cache_request};
#[cfg(test)]
use session::{reasoning_request_for_model, resolve_reasoning_request};

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCallId,
    UnsavedImagePart, UnsavedMessagePart,
};
use crate::error;
use crate::input::{ImageInput, read_image_input, validate_image_input_bytes};
use crate::llm::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::llm::{
    self, BaseUrl, BifrostClient, InputTokenCount, ModelInfo, ModelName, ModelParameter,
    ModelParameterOption, PromptCacheRequest, ReasoningRequest,
};
use crate::output::{RuntimeOutput, TerminalOutput};
use crate::runtime::context::ContextBuilder;
use crate::runtime::wakeup::Wakeup;
use crate::runtime::{
    PendingToolExecution, RuntimeEventSink, RuntimeInput, RuntimeModelRequest, RuntimeOutcome,
    advance_until_blocked as runtime_advance_until_blocked, deny_pending_tool_call,
    execute_pending_tool_call_with_catalog, load_pending_tool_call_at_head,
    pending_approvals_at_head, prepare_pending_tool_execution,
};
use crate::session::{
    Session, SessionCancellation, SessionControl, SessionEvent, SessionId, SessionStatus,
};
use crate::store::{Compaction, ConversationInfo, Store};
use crate::tool::ToolProviderRegistry;
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolApprovalRequest, ToolDefinition, ToolProviderId,
    ToolSchema, ToolSchemaName,
};

#[cfg(test)]
mod tests;
