//! Shared text and image parts for persisted conversation messages.
//!
//! These parts describe message content independently of the message role. User
//! messages and `role: tool` messages can both carry ordered text and image
//! content. Message ownership and assistant-tool-call relationships remain
//! represented by `Message::role` and message metadata.

use serde::{Deserialize, Serialize};

use crate::conversation::ImageAssetId;

#[derive(Debug, Clone, PartialEq, Eq)]
/// One typed piece of persisted model-facing message content.
pub enum MessagePart {
    Text(String),
    Image(ImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One typed message part before it has been copied into durable storage.
///
/// Unsaved parts carry raw bytes only. The store assigns durable asset IDs when
/// it writes an image-bearing message.
pub enum UnsavedMessagePart {
    Text(String),
    Image(UnsavedImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Durable image bytes attached to a message part.
pub struct ImagePart {
    pub asset_id: ImageAssetId,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Image bytes that have not yet been copied into durable image asset storage.
pub struct UnsavedImagePart {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
