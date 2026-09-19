//! Enrollment HTTP boundary. Browser, enrollment and device credentials never share authority.

use super::{
    HostedAccount, HostedStore,
    auth::HostedAuth,
    store::device::{CodeAction, DevicePrincipal, EnrollmentPrincipal},
};
use crate::device::*;
use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct Service {
    store: HostedStore,
    auth: HostedAuth,
    key: Option<Arc<Vec<u8>>>,
    trust_loopback_proxy: bool,
}

/// A missing key safely disables all new endpoints; existing hosted chat is unchanged.
pub(super) fn router(
    store: HostedStore,
    auth: HostedAuth,
    origin: HeaderValue,
) -> anyhow::Result<Router> {
    let key = std::env::var("WINDIE_DEVICE_ENROLLMENT_KEY")
        .ok()
        .map(|key| {
            anyhow::ensure!(
                valid_digest(&key),
                "WINDIE_DEVICE_ENROLLMENT_KEY must be 64 lowercase hex characters"
            );
            Ok(Arc::new(key.into_bytes()))
        })
        .transpose()?;
    let service = Service {
        store,
        auth,
        key,
        trust_loopback_proxy: std::env::var("WINDIE_DEVICE_TRUST_LOOPBACK_PROXY").as_deref()
            == Ok("1"),
    };
    if service.key.is_some() {
        let store = service.store.clone();
        tokio::spawn(async move {
            loop {
                if store.cleanup_devices().await.is_err() {
                    eprintln!("device retention cleanup unavailable");
                }
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
    }
    Ok(routes(service).layer(
        CorsLayer::new()
            .allow_origin(origin)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
            ]),
    ))
}
fn routes(service: Service) -> Router {
    Router::new()
        .route("/v1/device-enrollments", post(initiate))
        .route("/v1/device-enrollments/lookup", post(lookup))
        .route("/v1/device-enrollments/approve", post(approve))
        .route("/v1/device-enrollments/deny", post(deny))
        .route("/v1/device-enrollments/{id}", get(poll))
        .route("/v1/device-enrollments/{id}/finalize", post(finalize))
        .route("/v1/device-enrollments/{id}/cancel", post(cancel))
        .route("/v1/devices", get(list))
        .route("/v1/devices/{id}/revoke", post(revoke))
        .route("/v1/agent/self", get(own_device))
        .route("/v1/agent/connect", post(connect))
        .route("/v1/agent/heartbeat", post(heartbeat))
        .route("/v1/agent/disconnect", post(disconnect))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(service.clone(), guard))
        .with_state(service)
}
/// Limit requests before JSON extraction and auth work. Forwarded IPs are opt-in
/// only for a loopback tunnel connector configured to overwrite CF-Connecting-IP.
async fn guard(State(s): State<Service>, request: Request, next: Next) -> Response {
    let response = if s.key.is_none() {
        DeviceError::Unavailable.into_response()
    } else {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|p| p.0.ip());
        let ip = if s.trust_loopback_proxy && peer.is_some_and(|p| p.is_loopback()) {
            request
                .headers()
                .get("cf-connecting-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<std::net::IpAddr>().ok())
                .or(peer)
        } else {
            peer
        };
        let source = hash(
            &ip.map(|p| p.to_string())
                .unwrap_or_else(|| "unknown".into()),
        );
        let initiation = request.uri().path() == "/v1/device-enrollments";
        let bucket = format!(
            "{}:{source}",
            if initiation { "init" } else { "device-http" }
        );
        // A shared ceiling bounds per-source bucket growth even with rotating IPs.
        let limit = match s.store.device_rate_limit("device-global", 6000, 60).await {
            Ok(()) => {
                s.store
                    .device_rate_limit(&bucket, if initiation { 5 } else { 120 }, 60)
                    .await
            }
            Err(e) => Err(e),
        };
        match limit {
            Ok(()) => next.run(request).await,
            Err(e) => e.into_response(),
        }
    };
    let mut response = response;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
impl IntoResponse for DeviceError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Expired => StatusCode::GONE,
            Self::Conflict | Self::AlreadyRunning | Self::StaleLease => StatusCode::CONFLICT,
            Self::UnsupportedProtocol => StatusCode::UPGRADE_REQUIRED,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        };
        let mut response = (
            status,
            Json(serde_json::json!({"error":self,"message":self.to_string()})),
        )
            .into_response();
        if self == Self::RateLimited {
            response
                .headers_mut()
                .insert("retry-after", HeaderValue::from_static("60"));
        }
        response
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str, DeviceError> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(DeviceError::Unauthorized)
}
fn enrollment(headers: &HeaderMap, id: EnrollmentId) -> Result<EnrollmentPrincipal, DeviceError> {
    Ok(EnrollmentPrincipal {
        id,
        digest: secret_digest("enroll", bearer(headers)?)?,
    })
}
fn device(headers: &HeaderMap) -> Result<DevicePrincipal, DeviceError> {
    Ok(DevicePrincipal {
        digest: secret_digest("device", bearer(headers)?)?,
    })
}
impl Service {
    fn key(&self) -> Result<&[u8], DeviceError> {
        self.key
            .as_deref()
            .map(Vec::as_slice)
            .ok_or(DeviceError::Unavailable)
    }
    async fn account(&self, headers: &HeaderMap) -> Result<HostedAccount, DeviceError> {
        if bearer(headers)?.starts_with("wd_") {
            return Err(DeviceError::Unauthorized);
        }
        let subject = self.auth.authenticate(headers).await.map_err(|e| match e {
            super::auth::HostedAuthError::Unavailable => DeviceError::Unavailable,
            _ => DeviceError::Unauthorized,
        })?;
        self.store
            .resolve_account(&subject.0)
            .await
            .map_err(|_| DeviceError::Unavailable)
    }
    async fn code(
        &self,
        headers: &HeaderMap,
        code: &str,
        action: CodeAction,
    ) -> Result<EnrollmentView, DeviceError> {
        let account = self.account(headers).await?;
        self.store
            .device_rate_limit(&format!("code:{}", hash(&account.id)), 10, 60)
            .await?;
        // Stable verified subject is shown both in browser and terminal. No client label is trusted.
        self.store
            .device_code_action(&account, &account.auth_subject, code, action, self.key()?)
            .await
    }
}
async fn initiate(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<EnrollmentRequest>,
) -> Result<Json<EnrollmentStarted>, DeviceError> {
    // Public initiation has no bearer authority; never accidentally forward a device/browser token.
    if h.contains_key("authorization") {
        return Err(DeviceError::Unauthorized);
    }
    Ok(Json(s.store.initiate_device(&r, s.key()?).await?))
}
async fn lookup(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<CodeRequest>,
) -> Result<Json<EnrollmentView>, DeviceError> {
    Ok(Json(s.code(&h, &r.code, CodeAction::Lookup).await?))
}
async fn approve(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<CodeRequest>,
) -> Result<Json<EnrollmentView>, DeviceError> {
    Ok(Json(s.code(&h, &r.code, CodeAction::Approve).await?))
}
async fn deny(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<CodeRequest>,
) -> Result<Json<EnrollmentView>, DeviceError> {
    Ok(Json(s.code(&h, &r.code, CodeAction::Deny).await?))
}
async fn poll(
    State(s): State<Service>,
    h: HeaderMap,
    Path(id): Path<EnrollmentId>,
) -> Result<Json<EnrollmentView>, DeviceError> {
    Ok(Json(
        s.store
            .poll_device_enrollment(&enrollment(&h, id)?, s.key()?)
            .await?,
    ))
}
#[derive(Serialize)]
struct DeviceCreated {
    device_id: DeviceId,
}
async fn finalize(
    State(s): State<Service>,
    h: HeaderMap,
    Path(id): Path<EnrollmentId>,
    Json(r): Json<FinalizeRequest>,
) -> Result<Json<DeviceCreated>, DeviceError> {
    Ok(Json(DeviceCreated {
        device_id: s
            .store
            .finalize_device(&enrollment(&h, id)?, &r.account_id, s.key()?)
            .await?,
    }))
}
async fn cancel(
    State(s): State<Service>,
    h: HeaderMap,
    Path(id): Path<EnrollmentId>,
) -> Result<StatusCode, DeviceError> {
    s.store
        .cancel_device_enrollment(&enrollment(&h, id)?, s.key()?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list(
    State(s): State<Service>,
    h: HeaderMap,
) -> Result<Json<Vec<DeviceView>>, DeviceError> {
    Ok(Json(s.store.list_devices(&s.account(&h).await?).await?))
}
async fn revoke(
    State(s): State<Service>,
    h: HeaderMap,
    Path(id): Path<DeviceId>,
) -> Result<StatusCode, DeviceError> {
    s.store.revoke_device(&s.account(&h).await?, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn own_device(
    State(s): State<Service>,
    h: HeaderMap,
) -> Result<Json<DeviceView>, DeviceError> {
    Ok(Json(s.store.device_self(&device(&h)?).await?))
}
async fn connect(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<ConnectRequest>,
) -> Result<Json<Lease>, DeviceError> {
    Ok(Json(s.store.connect_device(&device(&h)?, &r).await?))
}
async fn heartbeat(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<LeaseRequest>,
) -> Result<Json<Lease>, DeviceError> {
    Ok(Json(
        s.store
            .heartbeat_device(&device(&h)?, r.lease_id, false)
            .await?,
    ))
}
async fn disconnect(
    State(s): State<Service>,
    h: HeaderMap,
    Json(r): Json<LeaseRequest>,
) -> Result<Json<Lease>, DeviceError> {
    Ok(Json(
        s.store
            .heartbeat_device(&device(&h)?, r.lease_id, true)
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn call(
        app: &Router,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut r = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            r = r.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(r.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let h = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        (
            status,
            h,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    #[tokio::test]
    #[ignore = "requires isolated WINDIE_HOSTED_TEST_DATABASE_URL (windie_test)"]
    async fn postgres_device_http_acceptance() {
        let fixture = super::super::store::device::tests::Fixture::new().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let auth_server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/auth/v1/user",
                    get(|h: HeaderMap| async move {
                        match bearer(&h) {
                            Ok("account-a") => (StatusCode::OK, Json(json!({"id":"verified-a"}))),
                            Ok("account-b") => (StatusCode::OK, Json(json!({"id":"verified-b"}))),
                            _ => (StatusCode::UNAUTHORIZED, Json(json!({}))),
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let service = Service {
            store: fixture.store.clone(),
            auth: HostedAuth::new(format!("http://{address}"), "test".into()),
            key: Some(Arc::new(b"test-key".to_vec())),
            trust_loopback_proxy: false,
        };
        let app = routes(service.clone());
        let enroll_secret = new_secret("enroll").unwrap();
        let device_secret = new_secret("device").unwrap();
        let mut request = super::super::store::device::tests::request();
        request.enrollment_digest = secret_digest("enroll", &enroll_secret).unwrap();
        request.device_digest = secret_digest("device", &device_secret).unwrap();
        let body = serde_json::to_value(&request).unwrap();
        let (status, h, start) = call(
            &app,
            Method::POST,
            "/v1/device-enrollments",
            None,
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(h["cache-control"], "no-store");
        let start: EnrollmentStarted = serde_json::from_value(start).unwrap();
        let (status, _, _) = call(
            &app,
            Method::GET,
            &format!("/v1/device-enrollments/{}", start.id),
            Some(&start.code),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        for token in [&enroll_secret, &device_secret] {
            let (status, _, _) =
                call(&app, Method::GET, "/v1/devices", Some(token), Value::Null).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        for token in [&enroll_secret, "account-a"] {
            assert_eq!(
                call(
                    &app,
                    Method::GET,
                    "/v1/agent/self",
                    Some(token),
                    Value::Null
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED
            );
        }
        let code = json!({"code":start.code});
        let (_, _, preview) = call(
            &app,
            Method::POST,
            "/v1/device-enrollments/lookup",
            Some("account-a"),
            code.clone(),
        )
        .await;
        assert_eq!(preview["state"], "pending");
        assert!(preview["account_id"].is_null());
        let (_, _, approved) = call(
            &app,
            Method::POST,
            "/v1/device-enrollments/approve",
            Some("account-a"),
            code.clone(),
        )
        .await;
        assert_eq!(approved["account_label"], "verified-a");
        assert_eq!(
            call(
                &app,
                Method::POST,
                "/v1/device-enrollments/approve",
                Some("account-b"),
                code.clone()
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        let final_path = format!("/v1/device-enrollments/{}/finalize", start.id);
        let (status, _, created) = call(
            &app,
            Method::POST,
            &final_path,
            Some(&enroll_secret),
            json!({"account_id":approved["account_id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            call(
                &app,
                Method::GET,
                "/v1/agent/self",
                Some(&device_secret),
                Value::Null
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &app,
                Method::GET,
                "/v1/devices",
                Some("account-b"),
                Value::Null
            )
            .await
            .2,
            json!([])
        );
        let revoke_path = format!(
            "/v1/devices/{}/revoke",
            created["device_id"].as_str().unwrap()
        );
        assert_eq!(
            call(
                &app,
                Method::POST,
                &revoke_path,
                Some("account-b"),
                Value::Null
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(
                &app,
                Method::POST,
                &revoke_path,
                Some("account-a"),
                Value::Null
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(
                &app,
                Method::GET,
                "/v1/agent/self",
                Some(&device_secret),
                Value::Null
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        // An active device credential cannot become browser or initiation authority.
        assert_eq!(
            call(
                &app,
                Method::POST,
                "/v1/device-enrollments",
                Some(&device_secret),
                body.clone()
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        for _ in 0..3 {
            call(
                &app,
                Method::POST,
                "/v1/device-enrollments",
                None,
                body.clone(),
            )
            .await;
        }
        let (status, h, _) = call(
            &routes(service.clone()),
            Method::POST,
            "/v1/device-enrollments",
            None,
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(h["retry-after"], "60");
        // No proxy header can change the unknown/direct peer source when trust is off.
        let huge = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/device-enrollments/lookup")
            .header("content-type", "application/json")
            .body(Body::from("x".repeat(5000)))
            .unwrap();
        let response = app.clone().oneshot(huge).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let disabled = routes(Service {
            key: None,
            ..service
        });
        assert_eq!(
            call(
                &disabled,
                Method::GET,
                "/v1/devices",
                Some("account-a"),
                Value::Null
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        auth_server.abort();
        drop(app);
        fixture.finish().await;
    }
    #[test]
    fn principal_headers_do_not_cross() {
        let mut h = HeaderMap::new();
        h.insert(
            "authorization",
            format!("Bearer {}", new_secret("enroll").unwrap())
                .parse()
                .unwrap(),
        );
        assert!(device(&h).is_err());
        assert!(enrollment(&h, EnrollmentId::new()).is_ok());
        h.insert(
            "authorization",
            format!("Bearer {}", new_secret("device").unwrap())
                .parse()
                .unwrap(),
        );
        assert!(enrollment(&h, EnrollmentId::new()).is_err());
        assert!(device(&h).is_ok());
    }
}
