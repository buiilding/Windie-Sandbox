//! Reusable, directory-shaped instructions that guide supported workflows.
//!
//! Skills are not executors. They are bounded instruction assets that plugins
//! reference and the model can load through Windie's builtin `read_skill` tool.

mod embedded;
mod manifest;
mod path;
mod registry;

pub use manifest::{SkillBundle, SkillDocument, SkillFile, SkillId, SkillManifest};
pub use path::SkillPath;
pub use registry::SkillRegistry;
