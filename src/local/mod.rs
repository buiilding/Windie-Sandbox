//! User-local Windie environment boundary.
//!
//! This folder owns files and commands tied to the local user's Windie runtime
//! environment, such as `~/.windie`, provider-key env editing, and
//! approved dependency checks.

pub mod process;
mod setup;
pub mod tray;

#[cfg(test)]
pub(crate) static ENVIRONMENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub use setup::{
    InstallReport, InstallStatus, UninstallCleanup, UninstallPlan, component_log_file_path,
    component_pid_file_path, ensure_windie_layout, env_file_path, env_value, install_target,
    list_env_keys, remove_uninstall_plan, set_env_values, uninstall_plan, unset_env_values,
    user_home_dir, windie_home_dir,
};
