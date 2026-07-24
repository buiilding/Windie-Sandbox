//! Terminal prompts for `windie onboard`.
//!
//! This module owns stdin/stdout interaction only. Provider configuration,
//! model discovery, secret persistence, and MCP lifecycle transitions remain
//! in the shared operation and provider boundaries.

use std::io::{self, Write};

use anyhow::{Result, anyhow};
use rpassword::prompt_password;

use crate::llm::{ProviderCatalog, ProviderCatalogEntry};
use crate::operation::{OnboardingPrompter, ProviderInstallation};
use crate::tool::ToolProviderId;

/// Interactive stdin/stdout adapter for the onboarding workflow.
pub struct TerminalOnboarding;

impl TerminalOnboarding {
    /// Creates a terminal onboarding adapter.
    pub fn new() -> Self {
        Self
    }

    fn prompt(&mut self, message: &str) -> Result<String> {
        print!("{message}");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    }

    fn choose_indexes(&mut self, count: usize) -> Result<Vec<usize>> {
        let input = self.prompt("Select numbers separated by commas, or press Enter to skip: ")?;
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let mut indexes = Vec::new();
        for value in input.split(',') {
            let number = value
                .trim()
                .parse::<usize>()
                .map_err(|_| anyhow!("invalid selection: {value}"))?;
            if number == 0 || number > count {
                return Err(anyhow!(
                    "selection is outside the available range: {number}"
                ));
            }
            let index = number - 1;
            if !indexes.contains(&index) {
                indexes.push(index);
            }
        }
        Ok(indexes)
    }
}

impl Default for TerminalOnboarding {
    fn default() -> Self {
        Self::new()
    }
}

impl OnboardingPrompter for TerminalOnboarding {
    fn choose_llm_providers(&mut self, catalog: &ProviderCatalog) -> Result<Vec<usize>> {
        println!();
        println!("Choose LLM providers:");
        for (index, provider) in catalog.providers.iter().enumerate() {
            let configured = if provider.configured {
                format!("configured, {} key(s)", provider.key_count)
            } else {
                "not configured".to_string()
            };
            println!(
                "  {:>2}. {:<24} {} · {}",
                index + 1,
                provider.display_name,
                provider.authentication,
                configured
            );
        }
        self.choose_indexes(catalog.providers.len())
    }

    fn read_llm_key(&mut self, provider: &ProviderCatalogEntry) -> Result<(String, String)> {
        println!();
        println!("{} API key", provider.display_name);
        let default_name = format!("windie-{}-{}", provider.name, provider.key_count + 1);
        let name = self.prompt(&format!("Key name [{default_name}]: "))?;
        let name = if name.is_empty() { default_name } else { name };
        let value = prompt_password("API key: ")?;
        if value.trim().is_empty() {
            return Err(anyhow!("API key cannot be empty"));
        }
        Ok((name, value))
    }

    fn choose_mcp_providers(
        &mut self,
        providers: &[ProviderInstallation],
    ) -> Result<Vec<ToolProviderId>> {
        println!();
        println!("Choose Windie extensions:");
        for (index, provider) in providers.iter().enumerate() {
            let state = provider
                .installation
                .as_ref()
                .map(|installation| format!("{:?}", installation.state).to_lowercase())
                .unwrap_or_else(|| "not installed".to_string());
            println!(
                "  {:>2}. {:<24} {} · {} · {}",
                index + 1,
                provider.manifest.display_name,
                format!("{:?}", provider.manifest.scope).to_lowercase(),
                provider.manifest.category,
                state
            );
        }

        self.choose_indexes(providers.len()).map(|indexes| {
            indexes
                .into_iter()
                .map(|index| providers[index].manifest.provider_id.clone())
                .collect()
        })
    }

    fn read_mcp_secret(
        &mut self,
        manifest: &crate::tool_provider::ProviderManifest,
        secret: &str,
    ) -> Result<String> {
        let description = manifest
            .secrets
            .iter()
            .find(|candidate| candidate.env_key == secret)
            .map(|candidate| candidate.description.as_str())
            .unwrap_or("required provider secret");
        println!();
        println!("{}", manifest.display_name);
        println!("{description}");
        let value = prompt_password(&format!("{secret}: "))?;
        if value.trim().is_empty() {
            return Err(anyhow!("{secret} cannot be empty"));
        }
        Ok(value)
    }

    fn progress(&mut self, message: &str) {
        println!("✓ {message}");
    }

    fn provider_configured(&mut self, provider: &ProviderCatalogEntry) {
        println!("✓ {} configured in Bifrost", provider.display_name);
    }

    fn provider_skipped(&mut self, provider: &ProviderCatalogEntry) {
        println!(
            "! {} needs structured provider configuration; skipped by terminal onboarding",
            provider.display_name
        );
    }

    fn mcp_configured(&mut self, provider: &ProviderInstallation) {
        println!("✓ {} enabled", provider.manifest.display_name);
    }

    fn models_available(&mut self, count: usize) {
        println!("✓ {count} models available");
    }
}
