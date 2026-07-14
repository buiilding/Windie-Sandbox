//! Core conversation data.
//!
//! This module exposes Windie's typed conversation identifiers, messages,
//! message parts, assistant metadata, tool schemas, and compaction identifiers.
//! It only models runtime data; storage, output, input parsing, and LLM
//! serialization belong to other modules.

pub mod compaction;
pub mod id;
pub mod message;
pub mod metadata;
pub mod part;
pub mod system_prompt;
pub mod tool_schema;

pub use compaction::CompactionId;
pub use id::{ConversationId, ImageAssetId, MessageId};
pub use message::{Message, Role};
pub use metadata::{MessageMetadata, TokenUsage, ToolCall, ToolCallId};
pub use part::{ImagePart, MessagePart, UnsavedImagePart, UnsavedMessagePart};
pub use system_prompt::{SystemPrompt, SystemPromptId};
pub use tool_schema::{ToolSchema, ToolSchemaName};
