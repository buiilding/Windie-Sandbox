//! Shared local endpoint configuration.
//!
//! Windie keeps the local gateway and API endpoints configurable through
//! environment variables so separate installations can run together.
//! Full URL/address variables take precedence over their port-only shortcuts.

use std::env;

/// Windie's hosted account service. The URL and publishable key are public
/// application configuration; they identify the Supabase project whose user
/// sessions a local runtime will accept.
const DEFAULT_AUTH_URL: &str = "https://dosrpwiiterwggicjpwn.supabase.co";
const DEFAULT_AUTH_PUBLISHABLE_KEY: &str = "sb_publishable_VvJDbTh01TcJw0w4S0kr_g_DAZeAg67";

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

/// Returns the HTTP URL used by the hosted Inspector to reach the local API.
pub fn api_url() -> String {
    format!("http://{}", api_address())
}

/// Returns the marketplace index used by the production API.
pub fn marketplace_index_url() -> String {
    non_empty_env("WINDIE_MARKETPLACE_INDEX_URL")
        .unwrap_or_else(|| "https://marketplace.windieos.com/index.json".to_string())
}

/// Returns the hosted account service used to authenticate Inspector sessions.
///
/// Release builds use Windie's production account service. Environment
/// overrides keep staging installations isolated without changing the runtime
/// contract.
pub fn auth_url() -> String {
    non_empty_env("WINDIE_AUTH_URL").unwrap_or_else(|| DEFAULT_AUTH_URL.to_string())
}

/// Returns the public key required by Supabase's authenticated user endpoint.
///
/// This is deliberately not a service-role credential and grants no elevated
/// access to the hosted database.
pub fn auth_publishable_key() -> String {
    non_empty_env("WINDIE_AUTH_PUBLISHABLE_KEY")
        .unwrap_or_else(|| DEFAULT_AUTH_PUBLISHABLE_KEY.to_string())
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_or_default(name: &str, default: &str) -> String {
    non_empty_env(name).unwrap_or_else(|| default.to_string())
}
