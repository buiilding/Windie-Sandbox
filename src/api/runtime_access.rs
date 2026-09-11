//! Runtime-access policies for the localhost Windie API and public demo.
//!
//! Normal access requires either a local capability or a hosted Inspector
//! account paired through a durable single-owner row. The explicit unsafe demo
//! policy bypasses those checks for a disposable remotely reachable runtime.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

const LOCAL_LAUNCH_CODE_TTL: Duration = Duration::from_secs(60);
const LOCAL_AUTHORIZATION_SCHEME: &str = "WindieLocal ";

/// Identity already verified by Windie's hosted account service.
#[derive(Debug, Clone)]
pub(super) struct AuthenticatedAccount {
    subject: String,
}

/// Authentication and authorization policy attached to one API server.
#[derive(Clone)]
pub(super) enum RuntimeAccessControl {
    /// Production policy: accept either a paired hosted account or a local
    /// Inspector session minted by this API process.
    HostedAndLocal {
        hosted: HostedAccountVerifier,
        local: LocalInspectorVerifier,
    },
    /// Explicit opt-in policy for the disposable public Windie demo. Every
    /// request is accepted without a credential or durable account pairing.
    UnsafePublicDemo,
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

#[derive(Clone)]
pub(super) struct LocalInspectorVerifier {
    local_component_token: String,
    state: Arc<Mutex<LocalInspectorState>>,
}

#[derive(Default)]
struct LocalInspectorState {
    launch_codes: HashMap<String, Instant>,
    session_tokens: HashSet<String>,
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

#[derive(Debug, Deserialize)]
pub(super) struct LocalAccessExchangeRequest {
    code: String,
}

#[derive(Debug, Serialize)]
pub(super) struct LocalAccessLaunchResponse {
    code: String,
}

#[derive(Debug, Serialize)]
pub(super) struct LocalAccessExchangeResponse {
    access_token: String,
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
    pub(super) fn hosted_and_local(local_component_token: String) -> Self {
        Self::HostedAndLocal {
            hosted: HostedAccountVerifier {
                http: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("Windie hosted-account HTTP client should initialize"),
                auth_url: crate::config::auth_url(),
                publishable_key: crate::config::auth_publishable_key(),
            },
            local: LocalInspectorVerifier::new(local_component_token),
        }
    }

    /// Builds the deliberately unauthenticated policy selected by the public
    /// demo environment flag.
    pub(super) fn unsafe_public_demo() -> Self {
        Self::UnsafePublicDemo
    }

    /// Keeps benchmark and route fixtures independent from a live account
    /// service. The real server never selects this test-only variant.
    pub(super) fn unrestricted_for_isolated_tests() -> Self {
        Self::UnrestrictedForIsolatedTests
    }

    #[cfg(test)]
    /// Points the production policy at a local mock Auth server for route tests.
    pub(super) fn hosted_for_tests(auth_url: String) -> Self {
        Self::HostedAndLocal {
            hosted: HostedAccountVerifier {
                http: reqwest::Client::new(),
                auth_url,
                publishable_key: "test-publishable-key".to_string(),
            },
            local: LocalInspectorVerifier::new("test-local-component-token".to_string()),
        }
    }

    async fn authenticate(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<AuthenticatedAccount, AuthenticationFailure> {
        match self {
            Self::HostedAndLocal { hosted, .. } => hosted.authenticate(headers).await,
            Self::UnsafePublicDemo => Ok(AuthenticatedAccount {
                subject: "unsafe-public-demo".to_string(),
            }),
            Self::UnrestrictedForIsolatedTests => Ok(AuthenticatedAccount {
                subject: "isolated-test-account".to_string(),
            }),
        }
    }

    fn is_unrestricted(&self) -> bool {
        matches!(
            self,
            Self::UnsafePublicDemo | Self::UnrestrictedForIsolatedTests
        )
    }

    /// Returns whether a local presentation component supplied the private
    /// credential for one of the internal notification streams.
    fn authenticates_local_component(&self, headers: &HeaderMap) -> bool {
        match self {
            Self::HostedAndLocal { local, .. } => local.authenticates_component(headers),
            Self::UnsafePublicDemo => true,
            Self::UnrestrictedForIsolatedTests => true,
        }
    }

    /// Returns whether this API process issued the volatile browser token.
    fn authenticates_local_inspector(&self, headers: &HeaderMap) -> bool {
        match self {
            Self::HostedAndLocal { local, .. } => local.authenticates_session(headers),
            Self::UnsafePublicDemo => true,
            Self::UnrestrictedForIsolatedTests => true,
        }
    }

    /// Issues a one-time code after middleware verifies the local component.
    fn issue_local_launch_code(&self) -> Option<String> {
        match self {
            Self::HostedAndLocal { local, .. } => Some(local.issue_launch_code()),
            Self::UnsafePublicDemo => None,
            Self::UnrestrictedForIsolatedTests => None,
        }
    }

    /// Consumes one launch code and replaces it with a volatile browser token.
    fn exchange_local_launch_code(&self, code: &str) -> Option<String> {
        match self {
            Self::HostedAndLocal { local, .. } => local.exchange_launch_code(code),
            Self::UnsafePublicDemo => None,
            Self::UnrestrictedForIsolatedTests => None,
        }
    }
}

impl LocalInspectorVerifier {
    fn new(local_component_token: String) -> Self {
        Self {
            local_component_token,
            state: Arc::new(Mutex::new(LocalInspectorState::default())),
        }
    }

    fn authenticates_component(&self, headers: &HeaderMap) -> bool {
        headers
            .get(crate::config::LOCAL_COMPONENT_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == self.local_component_token)
    }

    fn issue_launch_code(&self) -> String {
        let code = uuid::Uuid::new_v4().simple().to_string();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        state
            .launch_codes
            .retain(|_, issued_at| now.duration_since(*issued_at) < LOCAL_LAUNCH_CODE_TTL);
        state.launch_codes.insert(code.clone(), now);
        code
    }

    fn exchange_launch_code(&self, code: &str) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let issued_at = state.launch_codes.remove(code)?;
        if issued_at.elapsed() >= LOCAL_LAUNCH_CODE_TTL {
            return None;
        }

        let token = uuid::Uuid::new_v4().simple().to_string();
        state.session_tokens.insert(token.clone());
        Some(token)
    }

    fn authenticates_session(&self, headers: &HeaderMap) -> bool {
        let Some(token) = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix(LOCAL_AUTHORIZATION_SCHEME))
        else {
            return false;
        };
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.session_tokens.contains(token)
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

/// Applies the configured runtime-access policy after browser preflight.
///
/// Under the normal policy, everything that reveals, changes, or executes
/// runtime state requires either a verified paired account or a local token.
/// The explicit unsafe demo policy accepts every request.
pub(super) async fn authorize_runtime_request(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Response {
    if state.runtime_access.is_unrestricted() {
        request.extensions_mut().insert(AuthenticatedAccount {
            subject: match &state.runtime_access {
                RuntimeAccessControl::UnsafePublicDemo => "unsafe-public-demo".to_string(),
                _ => "isolated-test-account".to_string(),
            },
        });
        return next.run(request).await;
    }
    if public_runtime_route(request.method(), request.uri().path()) {
        return next.run(request).await;
    }

    if local_component_route(request.method(), request.uri().path())
        && state
            .runtime_access
            .authenticates_local_component(request.headers())
    {
        return next.run(request).await;
    }

    if state
        .runtime_access
        .authenticates_local_inspector(request.headers())
    {
        if runtime_pairing_route(request.method(), request.uri().path()) {
            return access_response(
                StatusCode::FORBIDDEN,
                "Local Inspector sessions do not manage hosted account pairing.",
            );
        }
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
            | (&Method::POST, "/api/runtime/local-access/exchange")
            | (&Method::OPTIONS, _)
    )
}

/// Internal event streams are available to the local notifier only after it
/// proves possession of the user-local component credential. They remain
/// protected from arbitrary loopback clients and do not require a hosted
/// account token because the notifier is a peer process of this API server.
fn local_component_route(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&Method::GET, "/api/events")
            | (&Method::GET, "/api/events/cursor")
            | (&Method::GET, "/api/dev/notifications")
            | (&Method::GET, "/api/dev/tray-notifications")
            | (&Method::POST, "/api/runtime/local-access/launch")
    )
}

/// Mints a short-lived launch code for a trusted local Windie component.
/// Middleware verifies the private component credential before entering this
/// handler, so the response never exposes that long-lived credential.
pub(super) async fn issue_local_access_launch(State(state): State<ApiState>) -> Response {
    let Some(code) = state.runtime_access.issue_local_launch_code() else {
        return access_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Local Inspector launch is unavailable under the isolated test policy.",
        );
    };
    Json(LocalAccessLaunchResponse { code }).into_response()
}

/// Exchanges a one-time fragment code for the volatile token used by browser
/// API and SSE requests. Invalid, expired, and consumed codes deliberately
/// produce the same response.
pub(super) async fn exchange_local_access_launch(
    State(state): State<ApiState>,
    Json(request): Json<LocalAccessExchangeRequest>,
) -> Response {
    let Some(access_token) = state
        .runtime_access
        .exchange_local_launch_code(request.code.trim())
    else {
        return access_response(
            StatusCode::UNAUTHORIZED,
            "This local Inspector launch link is invalid or expired. Run `windie inspector open` again.",
        );
    };
    Json(LocalAccessExchangeResponse { access_token }).into_response()
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
