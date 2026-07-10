//! Windie filesystem locations.
//!
//! Runtime state lives outside source checkouts so an installed Windie process
//! can safely build a development checkout without sharing mutable paths.

use std::env;
use std::path::PathBuf;

/// Returns Windie's per-user data directory.
///
/// Development and tests can set `WINDIE_DATA_DIR` to isolate all mutable
/// runtime state from the installed application.
pub fn data_dir() -> PathBuf {
    env::var_os("WINDIE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local").join("share").join("windie"))
}

/// Returns Windie's per-user configuration directory.
pub fn config_dir() -> PathBuf {
    env::var_os("WINDIE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config").join("windie"))
}

/// Returns the production conversation and runtime database path.
pub fn database_path() -> PathBuf {
    data_dir().join("windie.db")
}

/// Locates the bundled operator UI or a developer build when one exists.
pub fn operator_ui_dir() -> Option<PathBuf> {
    if let Some(path) = env::var_os("WINDIE_UI_DIR").map(PathBuf::from) {
        return path.is_dir().then_some(path);
    }

    if let Ok(executable) = env::current_exe() {
        let candidates = [std::fs::canonicalize(&executable).ok(), Some(executable)];
        for candidate in candidates.into_iter().flatten() {
            if let Some(directory) = candidate.parent() {
                let bundled = directory.join("ui");
                if bundled.is_dir() {
                    return Some(bundled);
                }
            }
        }
    }

    let developer_build = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("dev")
        .join("windie-inspector")
        .join("build");
    developer_build.is_dir().then_some(developer_build)
}

/// Returns a usable home directory fallback for early startup diagnostics.
fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
