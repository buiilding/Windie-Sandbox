//! Local user input loading boundary.
//!
//! This folder owns concrete input adapters before data enters conversation
//! storage. Runtime, API, and CLI code should depend on these typed helpers
//! instead of reading local user-provided files directly.

mod image;

pub use image::{ImageInput, read_image_input, validate_image_input_bytes};
