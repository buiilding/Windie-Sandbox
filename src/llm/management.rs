//! Bifrost management API client.
//!
//! This module owns the non-inference HTTP contract used to configure Bifrost
//! providers and keys. Windie does not persist LLM provider credentials; it
//! submits them to Bifrost's local management API and only keeps the returned
//! redacted metadata.

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// A provider returned by Bifrost's complete provider catalog.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProviderCatalogEntry {
    pub name: String,
    pub display_name: String,
    pub configured: bool,
    pub key_count: usize,
    pub authentication: String,
    pub configuration: String,
}

/// Complete provider catalog returned by Bifrost.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProviderCatalog {
    pub providers: Vec<ProviderCatalogEntry>,
    pub total: usize,
}

/// Minimal redacted key response needed by onboarding.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProviderKey {
    pub id: String,
    pub name: String,
}

/// Bifrost provider-key request for a simple API-key provider.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CreateProviderKey {
    pub name: String,
    pub value: String,
    pub models: Vec<String>,
    pub blacklisted_models: Vec<String>,
    pub weight: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AddProviderRequest<'a> {
    provider: &'a str,
}

/// HTTP client for Bifrost's local provider-management API.
pub struct BifrostManagementClient {
    http: Client,
    base_url: String,
}

impl BifrostManagementClient {
    /// Creates a management client bound to Bifrost's HTTP root, such as
    /// `http://localhost:8080`.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Loads every built-in and configured custom provider from Bifrost.
    pub async fn provider_catalog(&self) -> Result<ProviderCatalog> {
        self.http_json(
            self.http
                .get(format!("{}/api/providers/catalog", self.base_url)),
        )
        .await
    }

    /// Adds one simple API key to Bifrost's managed provider-key store.
    pub async fn create_provider_key(
        &self,
        provider: &str,
        request: &CreateProviderKey,
    ) -> Result<ProviderKey> {
        self.http_json(
            self.http
                .post(format!("{}/api/providers/{provider}/keys", self.base_url))
                .json(request),
        )
        .await
    }

    /// Creates Bifrost's default provider configuration before its first key
    /// is added. Bifrost's UI performs this same two-step flow.
    pub async fn ensure_provider(&self, provider: &str) -> Result<()> {
        let response = self
            .http
            .post(format!("{}/api/providers", self.base_url))
            .json(&AddProviderRequest { provider })
            .send()
            .await
            .context("failed to contact Bifrost provider API")?;
        if response.status().is_success() || response.status().as_u16() == 409 {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(anyhow!(
            "Bifrost provider creation failed with {status}: {body}"
        ))
    }

    async fn http_json<T>(&self, request: reqwest::RequestBuilder) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = request
            .send()
            .await
            .context("failed to contact Bifrost management API")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Bifrost management request failed with {status}: {body}"
            ));
        }

        response
            .json::<T>()
            .await
            .context("failed to parse Bifrost management response")
    }
}
