//! Shared terminal onboarding workflow.
//!
//! Onboarding coordinates Bifrost provider configuration with Windie's
//! existing MCP lifecycle operations. Prompt rendering stays in the CLI
//! adapter; this module owns startup, persistence, health checks, and cleanup.

use anyhow::Result;

use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::llm::{
    BaseUrl, BifrostManagementClient, CreateProviderKey, ProviderCatalog, ProviderCatalogEntry,
    list_models,
};
use crate::local;
use crate::store::Store;
use crate::tool::ToolProviderId;
use crate::tool_provider::{ProviderInstallState, ProviderManifest, ToolProviderRegistry};

use super::{
    ProviderInstallation, enable_provider, health_check_provider, list_provider_installations,
    repair_provider, setup_provider,
};

/// Interactive input and progress output required by onboarding clients.
pub trait OnboardingPrompter {
    /// Chooses provider indexes from the Bifrost-owned catalog.
    fn choose_llm_providers(&mut self, catalog: &ProviderCatalog) -> Result<Vec<usize>>;

    /// Reads a Bifrost managed-key name and secret for one provider.
    fn read_llm_key(&mut self, provider: &ProviderCatalogEntry) -> Result<(String, String)>;

    /// Chooses Windie MCP providers from their manifests and lifecycle state.
    fn choose_mcp_providers(
        &mut self,
        providers: &[ProviderInstallation],
    ) -> Result<Vec<ToolProviderId>>;

    /// Reads one manifest-declared MCP secret.
    fn read_mcp_secret(&mut self, manifest: &ProviderManifest, secret: &str) -> Result<String>;

    /// Prints non-secret progress information.
    fn progress(&mut self, message: &str);

    /// Prints one provider configuration result.
    fn provider_configured(&mut self, provider: &ProviderCatalogEntry);

    /// Prints that a provider needs a provider-specific configuration flow.
    fn provider_skipped(&mut self, provider: &ProviderCatalogEntry);

    /// Prints one MCP setup result.
    fn mcp_configured(&mut self, provider: &ProviderInstallation);

    /// Prints the final model count.
    fn models_available(&mut self, count: usize);
}

/// Runs onboarding and leaves Bifrost stopped only when this invocation started
/// it. Bifrost persists successfully submitted provider keys in its own store.
pub async fn run_onboarding<P: OnboardingPrompter>(
    prompter: &mut P,
    gateway_url: GatewayUrl,
    bifrost_url: &str,
    model_base_url: BaseUrl,
) -> Result<()> {
    local::ensure_windie_layout()?;
    let gateway = BifrostGateway::new(gateway_url);
    let started = gateway.start().await?;

    prompter.progress("Bifrost is ready.");
    let result = run_onboarding_steps(prompter, bifrost_url, model_base_url).await;

    if started == GatewayStart::Started {
        match gateway.stop().await {
            Ok(GatewayStop::Stopped | GatewayStop::NotRunning) => {}
            Err(error) => {
                if result.is_ok() {
                    return Err(error);
                }
            }
        }
    }

    result
}

async fn run_onboarding_steps<P: OnboardingPrompter>(
    prompter: &mut P,
    bifrost_url: &str,
    model_base_url: BaseUrl,
) -> Result<()> {
    let bifrost = BifrostManagementClient::new(bifrost_url);
    let catalog = bifrost.provider_catalog().await?;
    let selected_provider_indexes = prompter.choose_llm_providers(&catalog)?;

    for index in selected_provider_indexes {
        let Some(provider) = catalog.providers.get(index) else {
            continue;
        };

        if provider.authentication == "none" {
            prompter.provider_configured(provider);
            continue;
        }

        if provider.configuration != "simple" || provider.authentication != "api_key" {
            prompter.provider_skipped(provider);
            continue;
        }

        if !provider.configured {
            bifrost.ensure_provider(&provider.name).await?;
        }
        let (name, value) = prompter.read_llm_key(provider)?;
        bifrost
            .create_provider_key(
                &provider.name,
                &CreateProviderKey {
                    name,
                    value,
                    models: vec!["*".to_string()],
                    blacklisted_models: Vec::new(),
                    weight: 1.0,
                    enabled: true,
                },
            )
            .await?;
        prompter.provider_configured(provider);
    }

    let model_count = match list_models(model_base_url).await {
        Ok(models) => models.len(),
        Err(_) => 0,
    };
    prompter.models_available(model_count);

    let store = Store::open()?;
    let registry = ToolProviderRegistry::new();
    let installations = list_provider_installations(&store, &registry)?;
    let selected_mcp_ids = prompter.choose_mcp_providers(&installations)?;

    for provider_id in selected_mcp_ids {
        let Some(provider) = installations
            .iter()
            .find(|provider| provider.manifest.provider_id == provider_id)
        else {
            continue;
        };

        let assignments = provider
            .manifest
            .secrets
            .iter()
            .map(|secret| {
                prompter
                    .read_mcp_secret(&provider.manifest, &secret.env_key)
                    .map(|value| (secret.env_key.clone(), value))
            })
            .collect::<Result<Vec<_>>>()?;
        if !assignments.is_empty() {
            local::set_env_values(&assignments)?;
        }

        let current_state = provider
            .installation
            .as_ref()
            .map(|installation| installation.state);
        match current_state {
            None => {
                setup_provider(&store, &registry, &provider_id)?;
            }
            Some(ProviderInstallState::Installed | ProviderInstallState::Disabled) => {
                health_check_provider(&store, &registry, &provider_id)?;
                enable_provider(&store, &registry, &provider_id)?;
            }
            Some(ProviderInstallState::Broken) => {
                repair_provider(&store, &registry, &provider_id)?;
                health_check_provider(&store, &registry, &provider_id)?;
                enable_provider(&store, &registry, &provider_id)?;
            }
            Some(ProviderInstallState::Enabled) => {}
            Some(ProviderInstallState::Updating) => {
                repair_provider(&store, &registry, &provider_id)?;
                health_check_provider(&store, &registry, &provider_id)?;
                enable_provider(&store, &registry, &provider_id)?;
            }
        }

        let updated = list_provider_installations(&store, &registry)?
            .into_iter()
            .find(|installation| installation.manifest.provider_id == provider_id)
            .expect("selected provider must remain in registry");
        prompter.mcp_configured(&updated);
    }

    Ok(())
}
