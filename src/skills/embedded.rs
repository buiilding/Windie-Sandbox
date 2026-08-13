//! Compile-time embedded skill assets.

#[derive(Debug, Clone, Copy)]
pub(crate) struct EmbeddedSkillFile {
    pub(crate) path: &'static str,
    pub(crate) content: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EmbeddedSkillDefinition {
    pub(crate) skill_id: &'static str,
    pub(crate) files: &'static [EmbeddedSkillFile],
}

include!(concat!(env!("OUT_DIR"), "/windie_embedded_skills.rs"));
