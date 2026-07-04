//! Local image input loading.
//!
//! This module reads user-provided image files before they are persisted. It does
//! not know about CLI commands, SQLite, or provider requests.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

const MAX_IMAGE_INPUT_BYTES: u64 = 20 * 1024 * 1024;

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
    let byte_len = fs::metadata(path)
        .with_context(|| format!("failed to read image metadata: {}", path.display()))?
        .len();
    if byte_len > MAX_IMAGE_INPUT_BYTES {
        return Err(anyhow!(
            "image is too large: {} bytes exceeds {} bytes",
            byte_len,
            MAX_IMAGE_INPUT_BYTES
        ));
    }

    let bytes =
        fs::read(path).with_context(|| format!("failed to read image: {}", path.display()))?;
    validate_image_header(&mime_type, &bytes)
        .with_context(|| format!("invalid image header: {}", path.display()))?;

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

/// Verifies that bytes match the selected image MIME type.
fn validate_image_header(mime_type: &str, bytes: &[u8]) -> Result<()> {
    let valid = match mime_type {
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP",
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        _ => false,
    };

    if valid {
        Ok(())
    } else {
        Err(anyhow!("bytes do not match {mime_type}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn rejects_unknown_image_extension() {
        let error = read_image_input(Path::new("image.txt")).unwrap_err();

        assert!(error.to_string().contains("unsupported image type"));
    }

    #[test]
    fn reads_supported_png_header() {
        let path = temp_image_path("png");
        fs::write(&path, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();

        let image = read_image_input(&path).unwrap();

        assert_eq!(image.mime_type, "image/png");
        assert_eq!(
            image.bytes,
            vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_mismatched_image_header() {
        let path = temp_image_path("png");
        fs::write(&path, b"not a png").unwrap();

        let error = read_image_input(&path).unwrap_err();

        assert!(error.to_string().contains("invalid image header"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_oversized_image_before_reading_bytes() {
        let path = temp_image_path("png");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_IMAGE_INPUT_BYTES + 1).unwrap();

        let error = read_image_input(&path).unwrap_err();

        assert!(error.to_string().contains("image is too large"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn validates_supported_headers() {
        assert!(validate_image_header("image/jpeg", &[0xff, 0xd8, 0xff]).is_ok());
        assert!(validate_image_header("image/gif", b"GIF87a").is_ok());
        assert!(validate_image_header("image/gif", b"GIF89a").is_ok());
        assert!(validate_image_header("image/webp", b"RIFFxxxxWEBP").is_ok());
    }

    fn temp_image_path(extension: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

        env::temp_dir().join(format!(
            "windie-image-input-{}-{nanos}-{counter}.{extension}",
            std::process::id()
        ))
    }
}
