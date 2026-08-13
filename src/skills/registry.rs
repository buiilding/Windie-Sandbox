//! Curated skill catalog and bounded instruction-file loading.

use anyhow::Result;

use crate::error;

use super::embedded::EMBEDDED_SKILLS;
use super::manifest::{SkillBundle, SkillDocument, SkillFile, SkillId, SkillManifest};
use super::path::SkillPath;

#[derive(Debug, Clone)]
/// Registry of skills bundled with this Windie build.
pub struct SkillRegistry {
    pub(crate) skills: Vec<SkillBundle>,
}

impl SkillRegistry {
    /// Returns the reviewed skills shipped by this Windie build.
    pub fn curated() -> Self {
        let files = embedded_files("cua-driver");
        Self {
            skills: vec![SkillBundle {
                manifest: SkillManifest {
                    skill_id: SkillId::new("cua-driver"),
                    display_name: "CUA Driver workflow".to_string(),
                    description: "Guidance for using Windie's approved computer-control provider safely and deliberately.".to_string(),
                    entrypoint: SkillPath::entrypoint(),
                    files: files.iter().map(|file| file.path.clone()).collect(),
                },
                files,
            }],
        }
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

fn embedded_files(skill_id: &str) -> Vec<SkillFile> {
    let definition = EMBEDDED_SKILLS
        .iter()
        .find(|definition| definition.skill_id == skill_id)
        .unwrap_or_else(|| panic!("missing embedded curated skill: {skill_id}"));

    definition
        .files
        .iter()
        .map(|file| SkillFile {
            path: SkillPath::new(file.path).expect("invalid embedded skill file path"),
            content: file.content.to_string(),
        })
        .collect()
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
    fn curated_skill_is_a_directory_shaped_bundle() {
        let registry = SkillRegistry::curated();
        let skill = registry
            .skills()
            .find(|skill| skill.skill_id.as_str() == "cua-driver")
            .unwrap();

        assert_eq!(skill.entrypoint.as_str(), "SKILL.md");
        assert!(skill.files.iter().any(|path| path.as_str() == "MACOS.md"));
        assert!(skill.files.iter().any(|path| path.as_str() == "README.md"));
    }

    #[test]
    fn reads_entrypoint_and_supporting_reference_on_demand() {
        let registry = SkillRegistry::curated();
        let skill_id = SkillId::new("cua-driver");
        let macos = SkillPath::new("MACOS.md").unwrap();

        let entrypoint = registry.read(&skill_id, None).unwrap();
        assert_eq!(entrypoint.path.as_str(), "SKILL.md");
        assert!(entrypoint.content.contains("MACOS.md"));
        assert!(entrypoint.available_files.iter().any(|path| path == &macos));

        let reference = registry.read(&skill_id, Some(&macos)).unwrap();
        assert_eq!(reference.path, macos);
        assert!(reference.content.contains("macOS"));
    }

    #[test]
    fn unknown_skill_file_is_rejected() {
        let registry = SkillRegistry::curated();
        let unknown = SkillPath::new("missing.md").unwrap();

        assert!(
            registry
                .read(&SkillId::new("cua-driver"), Some(&unknown))
                .is_err()
        );
    }
}
