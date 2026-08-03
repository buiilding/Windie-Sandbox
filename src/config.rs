//! Shared local endpoint configuration.
//!
//! Windie keeps the local gateway, API, and Inspector endpoints configurable
//! through environment variables so separate installations can run together.
//! Full URL/address variables take precedence over their port-only shortcuts.

use std::env;

/// Default Bifrost gateway URL.
pub const DEFAULT_GATEWAY_URL: &str = "http://localhost:8080";
/// Default Windie API socket address.
pub const DEFAULT_API_ADDRESS: &str = "127.0.0.1:8787";
/// Default Inspector socket address.
pub const DEFAULT_INSPECTOR_ADDRESS: &str = "127.0.0.1:3000";

/// Returns the configured gateway URL.
pub fn gateway_url() -> String {
    non_empty_env("WINDIE_GATEWAY_URL").unwrap_or_else(|| {
        format!(
            "http://localhost:{}",
            env_or_default("WINDIE_GATEWAY_PORT", "8080")
        )
    })
}

/// Returns the configured API socket address.
pub fn api_address() -> String {
    non_empty_env("WINDIE_API_ADDRESS")
        .unwrap_or_else(|| format!("127.0.0.1:{}", env_or_default("WINDIE_API_PORT", "8787")))
}

/// Returns the configured Inspector socket address.
pub fn inspector_address() -> String {
    non_empty_env("WINDIE_INSPECTOR_ADDRESS").unwrap_or_else(|| {
        format!(
            "127.0.0.1:{}",
            env_or_default("WINDIE_INSPECTOR_PORT", "3000")
        )
    })
}

/// Returns the HTTP URL used by the Inspector to reach the local API.
pub fn api_url() -> String {
    format!("http://{}", api_address())
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_or_default(name: &str, default: &str) -> String {
    non_empty_env(name).unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_stable() {
        assert_eq!(DEFAULT_GATEWAY_URL, "http://localhost:8080");
        assert_eq!(DEFAULT_API_ADDRESS, "127.0.0.1:8787");
        assert_eq!(DEFAULT_INSPECTOR_ADDRESS, "127.0.0.1:3000");
    }
}
