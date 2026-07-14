//! Message part data.
//!
//! Parts allow one message to carry ordered text and local images. Durable image
//! storage is owned by `store.rs`; this module only defines the typed runtime
//! shapes for saved and unsaved parts.

use crate::conversation::ImageAssetId;

#[derive(Debug, Clone, PartialEq, Eq)]
/// One typed piece of a model-facing message.
pub enum MessagePart {
    Text(String),
    Image(ImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One typed message part before it has been copied into durable storage.
///
/// Unsaved parts carry raw bytes only. `store.rs` is responsible for assigning
/// durable asset IDs when it writes the message.
pub enum UnsavedMessagePart {
    Text(String),
    Image(UnsavedImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Durable image bytes attached to a message.
pub struct ImagePart {
    pub asset_id: ImageAssetId,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Image bytes that have not yet been copied into durable image asset storage.
pub struct UnsavedImagePart {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
