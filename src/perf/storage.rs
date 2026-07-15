//! Benchmark report file storage.

use super::*;

/// Reads a JSON benchmark report from disk.
pub fn read_report(path: &Path) -> Result<PerformanceReport> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read benchmark report {}", path.display()))?;
    let report = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse benchmark report {}", path.display()))?;

    Ok(report)
}

/// Writes a JSON benchmark report to disk.
pub fn write_report(path: &Path, report: &PerformanceReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text =
        serde_json::to_string_pretty(report).context("failed to serialize benchmark report")?;

    fs::write(path, format!("{text}\n"))
        .with_context(|| format!("failed to write benchmark report {}", path.display()))
}

/// Returns Windie's default persisted benchmark baseline path.
pub fn default_baseline_path() -> Result<PathBuf> {
    let Some(home) = env::var_os("HOME") else {
        return Err(anyhow::anyhow!("HOME is not set"));
    };

    Ok(PathBuf::from(home)
        .join(".windie")
        .join("benchmarks")
        .join("baseline.json"))
}
