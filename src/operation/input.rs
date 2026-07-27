//! Operation-level user input loading helpers.

use super::*;

/// One ordered message part accepted by client-facing insert operations.
///
/// Text parts are stored directly. Path images are read through `input::image`;
/// byte images arrive from local clients such as clipboard paste. Both image
/// forms are validated before storage copies bytes into SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageInputPart {
    Text(String),
    ImagePath(PathBuf),
    ImageBytes { mime_type: String, bytes: Vec<u8> },
}

#[derive(Debug, Clone)]
/// Input prepared for durable storage or later session execution.
pub struct PreparedMessageInput {
    pub content: String,
    pub parts: Vec<UnsavedMessagePart>,
}

/// Loaded version of one insert part.
pub(super) enum LoadedInsertPart {
    Text(String),
    Image(ImageInput),
}

/// Reads image parts through the image input boundary.
pub(super) fn load_insert_part(part: &MessageInputPart) -> Result<LoadedInsertPart> {
    match part {
        MessageInputPart::Text(text) => Ok(LoadedInsertPart::Text(text.clone())),
        MessageInputPart::ImagePath(path) => read_image_input(path).map(LoadedInsertPart::Image),
        MessageInputPart::ImageBytes { mime_type, bytes } => {
            validate_image_input_bytes(mime_type, bytes)?;
            Ok(LoadedInsertPart::Image(ImageInput {
                mime_type: mime_type.clone(),
                bytes: bytes.clone(),
            }))
        }
    }
}

/// Validates and loads user input before it is inserted or queued.
pub fn prepare_message_input(parts: &[MessageInputPart]) -> Result<PreparedMessageInput> {
    validate_insert_parts(parts)?;
    let loaded_parts = parts
        .iter()
        .map(load_insert_part)
        .collect::<Result<Vec<_>>>()?;
    let prepared_parts = loaded_parts
        .iter()
        .map(|part| match part {
            LoadedInsertPart::Text(text) => UnsavedMessagePart::Text(text.clone()),
            LoadedInsertPart::Image(image) => UnsavedMessagePart::Image(UnsavedImagePart {
                mime_type: image.mime_type.clone(),
                bytes: image.bytes.clone(),
            }),
        })
        .collect();

    Ok(PreparedMessageInput {
        content: insert_content(parts),
        parts: prepared_parts,
    })
}

/// Validates that an insert carries at least one meaningful user input.
pub(super) fn validate_insert_parts(parts: &[MessageInputPart]) -> Result<()> {
    if parts.is_empty() {
        return Err(error::invalid_request("message requires text or parts"));
    }
    if parts.iter().all(empty_text_part) {
        return Err(error::invalid_request(
            "message requires non-empty text or an image",
        ));
    }

    Ok(())
}

/// Returns whether a part contributes no content.
fn empty_text_part(part: &MessageInputPart) -> bool {
    match part {
        MessageInputPart::Text(text) => text.is_empty(),
        MessageInputPart::ImagePath(_) | MessageInputPart::ImageBytes { .. } => false,
    }
}

/// Builds the plain text preview stored in the message row.
pub(super) fn insert_content(parts: &[MessageInputPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            MessageInputPart::Text(text) => Some(text.as_str()),
            MessageInputPart::ImagePath(_) | MessageInputPart::ImageBytes { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
