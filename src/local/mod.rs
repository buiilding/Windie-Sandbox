//! User-local operating-environment boundary.
//!
//! This folder owns Windie's user-local filesystem layout, environment files,
//! detached component processes, and uninstall lifecycle. MCP-specific
//! installation and managed runtime provisioning live in `mcp/`.

mod process;
mod setup;

#[cfg(test)]
pub(crate) static ENVIRONMENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub use setup::{
    UninstallCleanup, UninstallPlan, component_log_file_path, component_pid_file_path,
    ensure_windie_layout, env_file_path, env_value, list_env_keys, remove_uninstall_plan,
    set_env_values, uninstall_plan, unset_env_values, user_home_dir, windie_home_dir,
};

pub use process::{
    ManagedComponent, ProcessReport, ProcessState, read_output, start_api, start_inspector,
    stop_api, stop_inspector, stop_tray, stop_windie_processes,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use process::{register_tray, unregister_tray};
