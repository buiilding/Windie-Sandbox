//! User-local Windie environment boundary.
//!
//! This folder owns files and commands tied to the local user's Windie runtime
//! environment, such as `~/.windie`, provider-key env editing, API tokens, and
//! approved dependency checks.

mod setup;

pub use setup::{
    InstallReport, ensure_api_token, ensure_windie_layout, env_file_path, env_value,
    inspector_log_file_path, install_target, is_llm_env_key, list_env_keys, set_env_values,
    unset_env_values,
};
