//! Shared CLI/API operation layer.
//!
//! This module owns the orchestration that should be identical across clients:
//! loading inspection snapshots, inserting messages, mutating conversation
//! state, and resolving explicit tool approvals. CLI and API code translate
//! inputs into these typed operations and translate returned values into their
//! own output formats.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::context::{ContextBuilder, ContextParts};
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCallId, ToolSchema,
    ToolSchemaName, UnsavedImagePart, UnsavedMessagePart,
};
use crate::error;
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::image_input::{ImageInput, read_image_input, validate_image_input_bytes};
use crate::llm::{
    self, BaseUrl, BifrostClient, InputTokenCount, InputTokenCountOutcome, ModelInfo, ModelName,
    ModelParameter, ModelParameterOption, PromptCacheRequest, ReasoningRequest,
};
use crate::output::RuntimeOutput;
use crate::runtime::{
    RuntimeEventSink, RuntimeModelRequest, approve_tool_call, approve_tool_call_with_registry,
    approve_tool_call_with_registry_and_message, deny_tool_call, deny_tool_call_with_message,
    pending_tool_approvals, pending_tool_approvals_with_registry,
    query_conversation_resolving_automatic_tools,
    query_conversation_resolving_automatic_tools_with_events,
};
use crate::store::{Compaction, ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolApprovalRequest, ToolDefinition, ToolExecutionResult,
    ToolProviderId,
};
use crate::tool_provider::ToolProviderRegistry;

/// One ordered message part accepted by client-facing insert operations.
///
/// Text parts are stored directly. Path images are read through `image_input.rs`;
/// byte images arrive from local clients such as clipboard paste. Both image
/// forms are validated before storage copies bytes into SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageInputPart {
    Text(String),
    ImagePath(PathBuf),
    ImageBytes { mime_type: String, bytes: Vec<u8> },
}

const SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE: &str = ".";

/// Full message tree plus the selected active node.
pub struct ConversationTree {
    pub messages: Vec<Message>,
    pub active_message_id: Option<MessageId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Source of a pre-query input-token count.
pub enum InputTokenCountSource {
    PrequeryInput,
    PrequerySyntheticInput,
}

impl InputTokenCountSource {
    /// Returns the stable API/UI label for this count source.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrequeryInput => "prequery_input",
            Self::PrequerySyntheticInput => "prequery_synthetic_input",
        }
    }
}

/// Read-only model-facing payload pieces prepared for input-token counting.
///
/// Loading these pieces is separate from the async Bifrost request so API
/// handlers can release SQLite state before awaiting network I/O.
pub struct InputTokenCountContext {
    model_messages: Vec<Message>,
    tool_schemas: Vec<ToolSchema>,
    source: InputTokenCountSource,
}

impl InputTokenCountContext {
    /// Returns whether the count uses real context input or synthetic input.
    pub fn source(&self) -> InputTokenCountSource {
        self.source
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Client-facing outcome for a pre-query input-token count request.
pub enum InputTokenCountResult {
    Count(InputTokenCount),
    Unsupported,
    EmptyContext,
}

#[derive(Debug, Serialize)]
/// Normalized model-parameter metadata used by developer clients.
///
/// Bifrost returns a richer raw parameter schema. Windie extracts only the
/// effort selector needed for runtime query controls and preserves the raw
/// response for inspection/debugging.
pub struct ModelRuntimeParameters {
    model: String,
    supports_reasoning: bool,
    supports_prompt_caching: bool,
    reasoning: Option<ReasoningParameter>,
    raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Effort selector derived from Bifrost model parameters.
pub struct ReasoningParameter {
    source: ReasoningParameterSource,
    options: Vec<ModelParameterOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Bifrost parameter source used to build a normalized effort selector.
pub enum ReasoningParameterSource {
    ReasoningEffort,
    OutputConfigEffort,
}

#[derive(Debug, Serialize)]
/// Machine-readable snapshot of one conversation's current runtime state.
///
/// CLI JSON and API inspection both serialize this same operation-level shape.
/// It deliberately summarizes image bytes instead of exposing raw image data.
pub struct InspectionReport {
    conversation_id: String,
    active_message_id: Option<String>,
    model: String,
    reasoning: Option<ReasoningRequest>,
    system_prompt: Option<String>,
    tool_approval_mode: ToolApprovalMode,
    tool_schemas: Vec<ToolSchema>,
    messages: Vec<InspectionMessage>,
    active_path: Vec<InspectionMessage>,
    model_context: Vec<InspectionMessage>,
    latest_compaction: Option<InspectionCompaction>,
}

impl InspectionReport {
    /// Builds the serializable inspection report from loaded runtime data.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: &ConversationId,
        active_message_id: Option<&MessageId>,
        model: &str,
        reasoning: Option<ReasoningRequest>,
        system_prompt: Option<String>,
        tool_approval_mode: ToolApprovalMode,
        tool_schemas: Vec<ToolSchema>,
        messages: Vec<Message>,
        active_path: Vec<Message>,
        model_context: Vec<Message>,
        latest_compaction: Option<Compaction>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.as_str().to_string(),
            active_message_id: active_message_id.map(|id| id.as_str().to_string()),
            model: model.to_string(),
            reasoning,
            system_prompt,
            tool_approval_mode,
            tool_schemas,
            messages: inspection_messages(messages),
            active_path: inspection_messages(active_path),
            model_context: inspection_messages(model_context),
            latest_compaction: latest_compaction.map(InspectionCompaction::from_compaction),
        }
    }
}

#[derive(Debug, Serialize)]
/// Serializable message shape for inspection JSON.
struct InspectionMessage {
    id: Option<String>,
    parent_message_id: Option<String>,
    role: Role,
    content: String,
    parts: Vec<InspectionMessagePart>,
    metadata: Option<MessageMetadata>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Serializable message part that never includes raw image bytes.
enum InspectionMessagePart {
    Text {
        text: String,
    },
    Image {
        asset_id: String,
        mime_type: String,
        byte_count: usize,
    },
}

#[derive(Debug, Serialize)]
/// Serializable compaction checkpoint shape for inspection JSON.
struct InspectionCompaction {
    id: String,
    conversation_id: String,
    through_message_id: String,
    content: String,
    created_at: i64,
}

impl InspectionCompaction {
    /// Converts a store compaction row into the public inspection shape.
    fn from_compaction(compaction: Compaction) -> Self {
        Self {
            id: compaction.id.as_str().to_string(),
            conversation_id: compaction.conversation_id.as_str().to_string(),
            through_message_id: compaction.through_message_id.as_str().to_string(),
            content: compaction.content,
            created_at: compaction.created_at,
        }
    }
}

/// Converts loaded runtime messages into the public inspection message shape.
fn inspection_messages(messages: Vec<Message>) -> Vec<InspectionMessage> {
    messages
        .into_iter()
        .map(|message| InspectionMessage {
            id: message.id.map(|id| id.as_str().to_string()),
            parent_message_id: message.parent_message_id.map(|id| id.as_str().to_string()),
            role: message.role,
            content: message.content,
            parts: inspection_message_parts(message.parts),
            metadata: message.metadata,
        })
        .collect()
}

/// Converts typed message parts while preserving order and hiding image bytes.
fn inspection_message_parts(parts: Vec<MessagePart>) -> Vec<InspectionMessagePart> {
    parts
        .into_iter()
        .map(|part| match part {
            MessagePart::Text(text) => InspectionMessagePart::Text { text },
            MessagePart::Image(image) => InspectionMessagePart::Image {
                asset_id: image.asset_id.as_str().to_string(),
                mime_type: image.mime_type,
                byte_count: image.bytes.len(),
            },
        })
        .collect()
}

/// Creates an empty persisted conversation with its default model.
mod conversations;
mod execution;
mod models;
mod tools;

pub use conversations::*;
pub use execution::*;
pub use models::*;
pub use tools::*;

#[cfg(test)]
pub(crate) use execution::resolve_reasoning_request;
#[cfg(test)]
pub(crate) use models::conversation_prompt_cache_request;

#[cfg(test)]
mod tests;
