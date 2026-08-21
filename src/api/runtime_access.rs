//! Hosted-account authorization for the localhost Windie API.
//!
//! A hosted Inspector session proves who is asking. A durable single-owner row
//! then proves that this particular local runtime was explicitly paired with
//! that account. Neither the hosted site nor Supabase receives access to the
//! local SQLite data, provider keys, or tools.

use super::*;

/// Identity already verified by Windie's hosted account service.
#[derive(Debug, Clone)]
pub(super) struct AuthenticatedAccount {
    subject: String,
}

/// Authentication and authorization policy attached to one API server.
#[derive(Clone)]
pub(super) enum RuntimeAccessControl {
    /// Production policy: authenticate every browser request with Supabase.
    Hosted(HostedAccountVerifier),
    /// Isolated benchmark and route-test policy. It must never be used by the
    /// process that binds the user's loopback API.
    UnrestrictedForIsolatedTests,
}

#[derive(Clone)]
pub(super) struct HostedAccountVerifier {
    http: reqwest::Client,
    auth_url: String,
    publishable_key: String,
}

#[derive(Debug, Deserialize)]
struct HostedUser {
    id: String,
}

#[derive(Debug)]
enum AuthenticationFailure {
    MissingBearerToken,
    InvalidBearerToken,
    AccountServiceUnavailable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeAccessState {
    Unpaired,
    Linked,
    OwnedByAnotherAccount,
}

#[derive(Debug, Serialize)]
pub(super) struct RuntimeAccessResponse {
    state: RuntimeAccessState,
    linked_at: Option<i64>,
}

impl RuntimeAccessControl {
    /// Builds the policy used by the real localhost API process.
    pub(super) fn hosted() -> Self {
        Self::Hosted(HostedAccountVerifier {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Windie hosted-account HTTP client should initialize"),
            auth_url: crate::config::auth_url(),
            publishable_key: crate::config::auth_publishable_key(),
        })
    }

    /// Keeps benchmark and route fixtures independent from a live account
    /// service. Production `serve` always selects [`Self::hosted`].
    pub(super) fn unrestricted_for_isolated_tests() -> Self {
        Self::UnrestrictedForIsolatedTests
    }

    #[cfg(test)]
    /// Points the production policy at a local mock Auth server for route tests.
    pub(super) fn hosted_for_tests(auth_url: String) -> Self {
        Self::Hosted(HostedAccountVerifier {
            http: reqwest::Client::new(),
            auth_url,
            publishable_key: "test-publishable-key".to_string(),
        })
    }

    async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<AuthenticatedAccount, AuthenticationFailure> {
        match self {
            Self::Hosted(verifier) => verifier.authenticate(headers).await,
            Self::UnrestrictedForIsolatedTests => Ok(AuthenticatedAccount {
                subject: "isolated-test-account".to_string(),
            }),
        }
    }

    fn is_unrestricted(&self) -> bool {
        matches!(self, Self::UnrestrictedForIsolatedTests)
    }
}

impl HostedAccountVerifier {
    /// Validates a browser access token against Supabase Auth instead of merely
    /// decoding its claims. This works for both legacy HS256 projects and new
    /// asymmetric signing-key projects without embedding a signing secret in
    /// Windie.
    async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<AuthenticatedAccount, AuthenticationFailure> {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with("Bearer "))
            .ok_or(AuthenticationFailure::MissingBearerToken)?;

        let response = self
            .http
            .get(format!(
                "{}/auth/v1/user",
                self.auth_url.trim_end_matches('/')
            ))
            .header("apikey", &self.publishable_key)
            .header(AUTHORIZATION, authorization)
            .send()
            .await
            .map_err(|_| AuthenticationFailure::AccountServiceUnavailable)?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(AuthenticationFailure::InvalidBearerToken);
        }
        if !response.status().is_success() {
            return Err(AuthenticationFailure::AccountServiceUnavailable);
        }

        let user = response
            .json::<HostedUser>()
            .await
            .map_err(|_| AuthenticationFailure::AccountServiceUnavailable)?;
        if user.id.trim().is_empty() {
            return Err(AuthenticationFailure::InvalidBearerToken);
        }

        Ok(AuthenticatedAccount { subject: user.id })
    }
}

/// Protects local runtime routes after CORS handles browser preflight.
///
/// Health and shutdown retain their loopback-only lifecycle role. Everything
/// that reveals, changes, or executes runtime state requires a verified hosted
/// account and a matching local pairing.
pub(super) async fn authorize_runtime_request(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Response {
    if state.runtime_access.is_unrestricted() {
        request.extensions_mut().insert(AuthenticatedAccount {
            subject: "isolated-test-account".to_string(),
        });
        return next.run(request).await;
    }
    if public_runtime_route(request.method(), request.uri().path()) {
        return next.run(request).await;
    }

    let account = match state.runtime_access.authenticate(request.headers()).await {
        Ok(account) => account,
        Err(failure) => return authentication_failure_response(failure),
    };

    let is_pairing_route = runtime_pairing_route(request.method(), request.uri().path());
    if !is_pairing_route {
        let access = match open_store(&state).and_then(|store| store.runtime_access()) {
            Ok(access) => access,
            Err(error) => return ApiError::from(error).into_response(),
        };
        match access {
            Some(access) if access.account_id == account.subject => {}
            Some(_) => {
                return access_response(
                    StatusCode::FORBIDDEN,
                    "This local Windie runtime is paired with a different account.",
                );
            }
            None => {
                return access_response(
                    StatusCode::CONFLICT,
                    "This local Windie runtime has not been paired yet. Approve pairing in the hosted Inspector.",
                );
            }
        }
    }

    request.extensions_mut().insert(account);
    next.run(request).await
}

fn public_runtime_route(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&Method::GET, "/api/health")
            | (&Method::GET, "/api/status")
            | (&Method::POST, "/api/shutdown")
            | (&Method::OPTIONS, _)
    )
}

fn runtime_pairing_route(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&Method::GET, "/api/runtime/access") | (&Method::POST, "/api/runtime/access")
    )
}

fn authentication_failure_response(failure: AuthenticationFailure) -> Response {
    match failure {
        AuthenticationFailure::MissingBearerToken | AuthenticationFailure::InvalidBearerToken => {
            access_response(
                StatusCode::UNAUTHORIZED,
                "Sign in to Windie before connecting to this local runtime.",
            )
        }
        AuthenticationFailure::AccountServiceUnavailable => access_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Windie could not verify your account. Check your internet connection and try again.",
        ),
    }
}

fn access_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.to_string(),
            causes: vec![message.to_string()],
        }),
    )
        .into_response()
}

/// Returns the calling account's current pairing state without exposing the
/// local owner's identity to another hosted account.
pub(super) async fn runtime_access_status(
    State(state): State<ApiState>,
    Extension(account): Extension<AuthenticatedAccount>,
) -> ApiResult<RuntimeAccessResponse> {
    let response = match open_store(&state)?.runtime_access()? {
        None => RuntimeAccessResponse {
            state: RuntimeAccessState::Unpaired,
            linked_at: None,
        },
        Some(access) if access.account_id == account.subject => RuntimeAccessResponse {
            state: RuntimeAccessState::Linked,
            linked_at: Some(access.linked_at),
        },
        Some(_) => RuntimeAccessResponse {
            state: RuntimeAccessState::OwnedByAnotherAccount,
            linked_at: None,
        },
    };
    Ok(Json(response))
}

/// Persists the user's explicit approval to connect their hosted account to
/// this local runtime. A different account can never overwrite an owner.
pub(super) async fn pair_runtime_access(
    State(state): State<ApiState>,
    Extension(account): Extension<AuthenticatedAccount>,
) -> ApiResult<RuntimeAccessResponse> {
    let response = match open_store(&state)?.link_runtime_access(&account.subject)? {
        crate::store::RuntimeAccessLink::Linked(access)
        | crate::store::RuntimeAccessLink::AlreadyLinked(access) => RuntimeAccessResponse {
            state: RuntimeAccessState::Linked,
            linked_at: Some(access.linked_at),
        },
        crate::store::RuntimeAccessLink::OwnedByAnotherAccount => {
            return Err(crate::error::conflict(
                "This local Windie runtime is already paired with a different account.",
            )
            .into());
        }
    };
    Ok(Json(response))
}

/// Allows the current owner to deliberately remove their local pairing.
pub(super) async fn unpair_runtime_access(
    State(state): State<ApiState>,
    Extension(account): Extension<AuthenticatedAccount>,
) -> ApiResult<RuntimeAccessResponse> {
    let removed = open_store(&state)?.unlink_runtime_access(&account.subject)?;
    if !removed {
        return Err(
            crate::error::conflict("This account does not own the local Windie runtime.").into(),
        );
    }
    Ok(Json(RuntimeAccessResponse {
        state: RuntimeAccessState::Unpaired,
        linked_at: None,
    }))
}
