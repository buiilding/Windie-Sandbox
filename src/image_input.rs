//! Local image input loading.
//!
//! This module reads user-provided image files before they are persisted. It does
//! not know about CLI commands, SQLite, or provider requests.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

#[derive(Debug)]
/// Image bytes and MIME type read from a local file before persistence.
pub struct ImageInput {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// Reads a supported local image file.
pub fn read_image_input(path: &Path) -> Result<ImageInput> {
    let mime_type = image_mime_type(path)
        .with_context(|| format!("unsupported image type: {}", path.display()))?;
    let bytes =
        fs::read(path).with_context(|| format!("failed to read image: {}", path.display()))?;

    Ok(ImageInput { mime_type, bytes })
}

/// Infers the small set of image MIME types Windie can send to Bifrost.
fn image_mime_type(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime_type = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    };

    Some(mime_type.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_image_extension() {
        let error = read_image_input(Path::new("image.txt")).unwrap_err();

        assert!(error.to_string().contains("unsupported image type"));
    }
}
