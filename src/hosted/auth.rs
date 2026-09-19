//! Supabase access-token verification for the hosted server.

use axum::http::{HeaderMap, header::AUTHORIZATION};
use reqwest::StatusCode;
use serde::Deserialize;

/// Authenticated Supabase subject, before it is mapped to a Windie account.
#[derive(Debug, Clone)]
pub(crate) struct SupabaseSubject(pub(crate) String);

/// Validates Supabase access tokens without embedding an Auth signing secret.
#[derive(Clone)]
pub(crate) struct HostedAuth {
    client: reqwest::Client,
    supabase_url: String,
    publishable_key: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostedAuthError {
    #[error("a Supabase Bearer access token is required")]
    MissingBearerToken,
    #[error("the Supabase access token is invalid or expired")]
    InvalidBearerToken,
    #[error("Supabase Auth could not be reached")]
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct SupabaseUser {
    id: String,
}

impl HostedAuth {
    /// Creates an Auth verifier for one configured Supabase project.
    pub(crate) fn new(supabase_url: String, publishable_key: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("hosted Supabase Auth client should initialize");
        Self {
            client,
            supabase_url,
            publishable_key,
        }
    }

    /// Calls Supabase Auth's authenticated user endpoint. This supports both
    /// legacy HS256 projects and newer asymmetric signing keys without placing
    /// any token-signing secret in the Windie process.
    pub(crate) async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> Result<SupabaseSubject, HostedAuthError> {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with("Bearer "))
            .ok_or(HostedAuthError::MissingBearerToken)?;

        // Device/enrollment bearers are never browser authority. Reject them
        // locally instead of forwarding a machine credential to Supabase.
        if authorization.starts_with("Bearer wd_") {
            return Err(HostedAuthError::InvalidBearerToken);
        }

        let response = self
            .client
            .get(format!(
                "{}/auth/v1/user",
                self.supabase_url.trim_end_matches('/')
            ))
            .header("apikey", &self.publishable_key)
            .header(AUTHORIZATION, authorization)
            .send()
            .await
            .map_err(|_| HostedAuthError::Unavailable)?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(HostedAuthError::InvalidBearerToken);
        }
        if !response.status().is_success() {
            return Err(HostedAuthError::Unavailable);
        }

        let user = response
            .json::<SupabaseUser>()
            .await
            .map_err(|_| HostedAuthError::Unavailable)?;
        if user.id.trim().is_empty() {
            return Err(HostedAuthError::InvalidBearerToken);
        }
        Ok(SupabaseSubject(user.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn device_credentials_never_reach_account_verification() {
        let auth = HostedAuth::new("http://127.0.0.1:1".into(), "unused".into());
        for kind in ["device", "enroll"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                AUTHORIZATION,
                format!("Bearer {}", crate::device::new_secret(kind).unwrap())
                    .parse()
                    .unwrap(),
            );
            assert!(matches!(
                auth.authenticate(&headers).await,
                Err(HostedAuthError::InvalidBearerToken)
            ));
        }
    }
}
