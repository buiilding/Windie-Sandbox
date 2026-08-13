//! Typed metadata and file bundles for reusable Windie skills.

use serde::{Deserialize, Serialize};

use super::path::SkillPath;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
/// Stable identity for one skill.
pub struct SkillId(String);

impl SkillId {
    /// Creates a typed skill identity.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the stable identifier at runtime and display boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Discoverable metadata and bounded files for one skill.
pub struct SkillManifest {
    pub skill_id: SkillId,
    pub display_name: String,
    pub description: String,
    /// File loaded by default when the model reads this skill.
    pub entrypoint: SkillPath,
    /// Files available inside the skill bundle.
    pub files: Vec<SkillPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One text asset inside a skill bundle.
pub struct SkillFile {
    pub path: SkillPath,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A skill manifest paired with its bounded instruction files.
pub struct SkillBundle {
    pub manifest: SkillManifest,
    pub files: Vec<SkillFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One skill file returned to the model.
pub struct SkillDocument {
    pub skill_id: SkillId,
    pub path: SkillPath,
    pub content: String,
    pub available_files: Vec<SkillPath>,
}
