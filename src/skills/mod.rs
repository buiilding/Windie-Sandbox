//! Reusable instructions that guide the model through supported workflows.
//!
//! Skills are not executors. They are bounded instruction assets that plugins
//! reference and the model can load through Windie's builtin `read_skill` tool.

mod manifest;
mod registry;

pub use manifest::{SkillId, SkillManifest};
pub use registry::SkillRegistry;
