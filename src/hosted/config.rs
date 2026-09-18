//! Environment configuration for the hosted server process.

use std::net::SocketAddr;

use anyhow::{Context, Result, anyhow};

/// Configuration that is intentionally specific to the hosted deployment.
///
/// Provider credentials remain in the private Bifrost deployment. The hosted
/// server only needs Bifrost's loopback OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct HostedConfig {
    /// Private PostgreSQL connection string for Windie-owned state.
    pub database_url: String,
    /// Supabase project URL used only to validate browser access tokens.
    pub supabase_url: String,
    /// Publishable Supabase key sent to the Auth `/user` endpoint.
    pub supabase_publishable_key: String,
    /// Network address on which the hosted API listens.
    pub address: SocketAddr,
    /// The one browser origin permitted by the initial hosted API CORS policy.
    pub allowed_origin: String,
    /// Private OpenAI-compatible Bifrost endpoint used by hosted workers.
    pub bifrost_base_url: String,
    /// Default model assigned when a hosted conversation does not name one.
    ///
    /// This stays deployment configuration rather than a hard-coded provider
    /// choice, so changing the hosted provider affects new conversations only.
    pub default_model: String,
}

impl HostedConfig {
    /// Reads required hosted-server configuration without falling back to any
    /// local-runtime defaults. A production process must make its boundaries
    /// explicit rather than accidentally using a developer's local settings.
    pub fn from_environment() -> Result<Self> {
        let database_url = required("WINDIE_HOSTED_DATABASE_URL")?;
        let supabase_url = required("WINDIE_HOSTED_SUPABASE_URL")?;
        let supabase_publishable_key = required("WINDIE_HOSTED_SUPABASE_PUBLISHABLE_KEY")?;
        let address = std::env::var("WINDIE_HOSTED_ADDRESS")
            .unwrap_or_else(|_| "127.0.0.1:8788".to_string())
            .parse()
            .context("WINDIE_HOSTED_ADDRESS must be a socket address")?;
        let allowed_origin = std::env::var("WINDIE_HOSTED_ALLOWED_ORIGIN")
            .unwrap_or_else(|_| "https://app.windieos.com".to_string());
        let bifrost_base_url = std::env::var("WINDIE_HOSTED_BIFROST_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080/v1".to_string());
        let default_model = std::env::var("WINDIE_HOSTED_DEFAULT_MODEL")
            .unwrap_or_else(|_| "windie/hosted".to_string());

        if !allowed_origin.starts_with("https://") && !allowed_origin.starts_with("http://") {
            return Err(anyhow!(
                "WINDIE_HOSTED_ALLOWED_ORIGIN must be an http or https origin"
            ));
        }
        if default_model.trim().is_empty() {
            return Err(anyhow!(
                "WINDIE_HOSTED_DEFAULT_MODEL must not be empty when configured"
            ));
        }

        Ok(Self {
            database_url,
            supabase_url,
            supabase_publishable_key,
            address,
            allowed_origin,
            bifrost_base_url,
            default_model,
        })
    }
}

fn required(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{name} must be configured"))
}
