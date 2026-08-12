//! Curated skill catalog and instruction loading.

use anyhow::Result;

use crate::error;

use super::manifest::{SkillId, SkillManifest};

const DRIVER_SKILL: &str = include_str!("curated/driver.md");

#[derive(Debug, Clone)]
/// Registry of skills bundled with this Windie build.
pub struct SkillRegistry {
    pub(crate) skills: Vec<SkillManifest>,
}

impl SkillRegistry {
    /// Returns the reviewed skills shipped by this Windie build.
    pub fn curated() -> Self {
        Self {
            skills: vec![SkillManifest {
                skill_id: SkillId::new("driver"),
                display_name: "Computer driver workflow".to_string(),
                description: "Guidance for using Windie's approved computer-control provider safely and deliberately.".to_string(),
                content: DRIVER_SKILL.to_string(),
            }],
        }
    }

    /// Returns every skill in deterministic catalog order.
    pub fn skills(&self) -> &[SkillManifest] {
        &self.skills
    }

    /// Loads the full instructions for one skill.
    pub fn read(&self, skill_id: &SkillId) -> Result<String> {
        self.skills
            .iter()
            .find(|skill| skill.skill_id == *skill_id)
            .map(|skill| skill.content.clone())
            .ok_or_else(|| error::not_found(format!("skill does not exist: {skill_id}")))
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::curated()
    }
}
