//! User-local Windie environment boundary.
//!
//! This folder owns files and commands tied to the local user's Windie runtime
//! environment, such as `~/.windie`, provider-key env editing, and
//! approved dependency checks.

mod runtime;
mod setup;

pub use setup::{
    InstallReport, component_log_file_path, component_pid_file_path, ensure_windie_layout,
    env_file_path, env_value, install_target, list_env_keys, set_env_values, unset_env_values,
    windie_home_dir,
};

pub(crate) use runtime::{path_with_command_parent, resolve_command};
