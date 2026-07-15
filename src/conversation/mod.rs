//! Core conversation data.
//!
//! This module exposes Windie's typed conversation identifiers, messages,
//! message parts, assistant metadata, and compaction identifiers.
//! It only models runtime data; storage, output, input parsing, and LLM
//! serialization belong to other modules.

pub mod assistant_metadata;
pub mod id;
pub mod message;
pub mod user_part;

pub use assistant_metadata::{MessageMetadata, TokenUsage, ToolCall, ToolCallId};
pub use id::{CompactionId, ConversationId, ImageAssetId, MessageId};
pub use message::{Message, Role};
pub use user_part::{ImagePart, MessagePart, UnsavedImagePart, UnsavedMessagePart};
