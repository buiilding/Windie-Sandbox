//! Curated skill catalog and bounded instruction-file loading.

use anyhow::Result;

use crate::error;

use super::manifest::{SkillBundle, SkillDocument, SkillId, SkillManifest};
use super::path::SkillPath;

#[derive(Debug, Clone)]
/// Registry of skill bundles that are owned by the current runtime.
///
/// Curated upstream skills are installed as file-based plugin packages and
/// loaded by `PluginRegistry`; this registry remains available for future
/// Windie-owned skills and test fixtures.
pub struct SkillRegistry {
    pub(crate) skills: Vec<SkillBundle>,
}

impl SkillRegistry {
    /// Returns Windie-owned skill fixtures compiled into this build.
    ///
    /// The current curated set has no Windie-owned skill files. Upstream skill
    /// packs are materialized into the user-local plugin store during
    /// installation.
    pub fn curated() -> Self {
        Self { skills: Vec::new() }
    }

    /// Returns every skill in deterministic catalog order.
    pub fn skills(&self) -> impl Iterator<Item = &SkillManifest> {
        self.skills.iter().map(|skill| &skill.manifest)
    }

    /// Loads one file from a skill, defaulting to its `SKILL.md` entrypoint.
    pub fn read(&self, skill_id: &SkillId, path: Option<&SkillPath>) -> Result<SkillDocument> {
        let skill = self
            .skills
            .iter()
            .find(|skill| skill.manifest.skill_id == *skill_id)
            .ok_or_else(|| error::not_found(format!("skill does not exist: {skill_id}")))?;
        let path = path.unwrap_or(&skill.manifest.entrypoint);
        let file = skill
            .files
            .iter()
            .find(|file| file.path == *path)
            .ok_or_else(|| {
                error::not_found(format!("skill file does not exist: {skill_id}/{path}"))
            })?;

        Ok(SkillDocument {
            skill_id: skill.manifest.skill_id.clone(),
            path: file.path.clone(),
            content: file.content.clone(),
            available_files: skill
                .manifest
                .files
                .iter()
                .filter(|candidate| candidate.as_str() != file.path.as_str())
                .cloned()
                .collect(),
        })
    }

    /// Creates a skill registry from already validated bundles.
    pub(crate) fn from_bundles(skills: Vec<SkillBundle>) -> Self {
        Self { skills }
    }
}

impl SkillDocument {
    /// Formats the document for the model while exposing reference names but
    /// not loading their contents into the same response.
    pub fn as_model_text(&self) -> String {
        let mut text = format!(
            "Skill: {}\nFile: {}\n\n{}",
            self.skill_id, self.path, self.content
        );
        if !self.available_files.is_empty() {
            text.push_str("\n\nAvailable supporting files:\n");
            for path in &self.available_files {
                text.push_str("- ");
                text.push_str(path.as_str());
                text.push('\n');
            }
        }
        text
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::curated()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_registry_does_not_embed_upstream_skill_content() {
        assert!(SkillRegistry::curated().skills().next().is_none());
    }
}
