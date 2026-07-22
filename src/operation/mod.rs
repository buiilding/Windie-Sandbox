//! Shared CLI/API operation layer.
//!
//! This module owns the orchestration that should be identical across clients:
//! loading inspection snapshots, inserting messages, mutating conversation
//! state, and resolving explicit tool approvals. CLI and API code translate
//! inputs into these typed operations and translate returned values into their
//! own output formats.

mod conversation;
mod gateway;
mod input;
mod inspection;
mod message;
mod session;
mod session_approval;
mod session_cli;
mod tool;

pub use conversation::*;
pub use gateway::*;
pub use input::{MessageInputPart, PreparedMessageInput, prepare_message_input};
pub use inspection::*;
pub use message::*;
pub use session::*;
pub use session_approval::*;
pub use session_cli::*;
pub use tool::*;

#[cfg(test)]
use gateway::{SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE, conversation_prompt_cache_request};
#[cfg(test)]
use session::{reasoning_request_for_model, resolve_reasoning_request};

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::context::ContextBuilder;
use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCallId,
    UnsavedImagePart, UnsavedMessagePart,
};
use crate::error;
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::input::{ImageInput, read_image_input, validate_image_input_bytes};
use crate::llm::{
    self, BaseUrl, BifrostClient, InputTokenCount, ModelInfo, ModelName, ModelParameter,
    ModelParameterOption, PromptCacheRequest, ReasoningRequest,
};
use crate::output::{RuntimeOutput, TerminalOutput};
use crate::runtime::{
    PendingToolExecution, RuntimeEventSink, RuntimeInput, RuntimeModelRequest, RuntimeOutcome,
    advance_until_blocked as runtime_advance_until_blocked, deny_pending_tool_call,
    execute_pending_tool_call, load_pending_tool_call_at_head, pending_approvals_at_head,
    prepare_pending_tool_execution, store_pending_tool_result_at_head,
};
use crate::session::{Session, SessionEvent, SessionId, SessionStatus};
use crate::store::{Compaction, ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolApprovalRequest, ToolDefinition, ToolProviderId,
    ToolSchema, ToolSchemaName,
};
use crate::tool_provider::ToolProviderRegistry;
use crate::wakeup::Wakeup;

#[cfg(test)]
mod tests;
