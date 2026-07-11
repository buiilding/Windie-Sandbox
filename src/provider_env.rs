//! Canonical provider secret environment.
//!
//! Windie and Bifrost resolve provider credentials from one dotenv file. An
//! explicit process variable overrides the matching file value without making
//! unrelated process environment available to provider children.

use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use crate::paths;

const PROVIDER_ENV_FILE_NAME: &str = "providers.env";

/// Loads every provider value from the first configured environment file.
///
/// Values already present in Windie's process environment take precedence.
/// Missing files are allowed so commands unrelated to providers can run
/// without initial configuration.
pub fn load() -> Result<Vec<(String, String)>> {
    let Some(path) = find_file_path() else {
        return Ok(Vec::new());
    };
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read provider environment {}", path.display()))?;
    let values = parse(&text)
        .with_context(|| format!("failed to parse provider environment {}", path.display()))?;

    Ok(values
        .into_iter()
        .map(|(key, file_value)| {
            let value = env::var(&key).unwrap_or(file_value);
            (key, value)
        })
        .collect())
}

/// Resolves one required provider value from process environment or the
/// canonical provider file.
pub fn required(name: &str) -> Result<String> {
    if let Ok(value) = env::var(name) {
        return Ok(value);
    }

    load()?
        .into_iter()
        .find_map(|(key, value)| (key == name).then_some(value))
        .ok_or_else(|| anyhow!("missing required provider environment variable: {name}"))
}

/// Finds the first supported provider environment file.
fn find_file_path() -> Option<PathBuf> {
    file_candidates().into_iter().find(|path| path.is_file())
}

/// Returns provider environment locations from most to least explicit.
fn file_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("WINDIE_ENV_FILE") {
        candidates.push(PathBuf::from(path));
    }

    candidates.push(paths::canonical_config_dir().join(PROVIDER_ENV_FILE_NAME));
    candidates
}

/// Parses simple dotenv assignments without evaluating shell syntax.
fn parse(text: &str) -> Result<Vec<(String, String)>> {
    let mut values = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            return Err(anyhow!("invalid provider environment line {}", index + 1));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!(
                "empty provider environment key on line {}",
                index + 1
            ));
        }
        values.push((key.to_string(), unquote(value.trim()).to_string()));
    }
    Ok(values)
}

/// Removes matching quotes around one complete dotenv value.
fn unquote(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_environment_values() {
        let values = parse(
            r#"
            # provider keys
            OPENAI_API_KEY=sk-test
            export ANTHROPIC_API_KEY='sk-ant-test'
            EXA_API_KEY="exa-test"
            EMPTY=
            "#,
        )
        .unwrap();

        assert_eq!(
            values,
            vec![
                ("OPENAI_API_KEY".to_string(), "sk-test".to_string()),
                ("ANTHROPIC_API_KEY".to_string(), "sk-ant-test".to_string()),
                ("EXA_API_KEY".to_string(), "exa-test".to_string()),
                ("EMPTY".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn rejects_invalid_provider_environment_line() {
        let error = parse("OPENAI_API_KEY").unwrap_err();

        assert_eq!(error.to_string(), "invalid provider environment line 1");
    }
}
