//! Typed metadata for reusable Windie skills.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
/// Discoverable metadata and bounded instructions for one skill.
pub struct SkillManifest {
    pub skill_id: SkillId,
    pub display_name: String,
    pub description: String,
    pub content: String,
}
