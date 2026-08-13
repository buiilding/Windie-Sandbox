//! Safe relative paths for files inside a skill bundle.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
/// A normalized, bundle-relative skill file path.
pub struct SkillPath(String);

impl SkillPath {
    /// Creates a safe bundle-relative path using `/` separators.
    pub fn new(path: impl Into<String>) -> Result<Self, String> {
        let path = path.into();
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(format!("invalid skill file path: {path}"));
        }
        Ok(Self(path))
    }

    /// Returns the default skill entrypoint.
    pub fn entrypoint() -> Self {
        Self("SKILL.md".to_string())
    }

    /// Returns the normalized path at display and lookup boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bundle_relative_paths() {
        assert!(SkillPath::new("MACOS.md").is_ok());
        assert!(SkillPath::new("references/setup.md").is_ok());
    }

    #[test]
    fn rejects_paths_that_escape_the_bundle() {
        assert!(SkillPath::new("../secret.txt").is_err());
        assert!(SkillPath::new("references/../../secret.txt").is_err());
        assert!(SkillPath::new("/Users/peter/secret.txt").is_err());
        assert!(SkillPath::new("references\\setup.md").is_err());
    }
}
