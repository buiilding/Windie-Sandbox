//! Outbound presence-only device agent. Never starts MCP, Bifrost, or a local session.

pub(crate) mod enrollment;
pub(crate) mod storage;
#[cfg(test)]
mod tests;
use crate::device::*;
use anyhow::{Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use storage::Credentials;

/// Transport failures carry safe categories, not URLs, credential headers or server bodies.
#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub(crate) struct AgentError {
    pub code: DeviceError,
    pub retry_after: u64,
}
impl AgentError {
    pub fn retryable(&self) -> bool {
        matches!(
            self.code,
            DeviceError::Unavailable | DeviceError::RateLimited
        )
    }
}
#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    pub server: String,
}
impl Client {
    pub fn new(server: &str) -> Result<Self> {
        let server = validate_server(server)?;
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(REQUEST_SECONDS))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            server,
        })
    }
    pub async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        secret: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> std::result::Result<T, AgentError> {
        let unavailable = || AgentError {
            code: DeviceError::Unavailable,
            retry_after: 0,
        };
        let mut request = self
            .http
            .request(method, format!("{}{}", self.server, path));
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request.send().await.map_err(|_| unavailable())?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > 65536 {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let code = if status.as_u16() == 401 {
                DeviceError::Unauthorized
            } else if status.as_u16() == 429 {
                DeviceError::RateLimited
            } else if status.is_server_error() {
                DeviceError::Unavailable
            } else {
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .and_then(|v| serde_json::from_value(v.get("error")?.clone()).ok())
                    .unwrap_or(DeviceError::InvalidRequest)
            };
            return Err(AgentError { code, retry_after });
        }
        serde_json::from_slice(if bytes.is_empty() { b"null" } else { &bytes }).map_err(|_| {
            AgentError {
                code: DeviceError::InvalidRequest,
                retry_after: 0,
            }
        })
    }
    pub async fn own_device(&self, c: &Credentials) -> std::result::Result<DeviceView, AgentError> {
        self.request(
            reqwest::Method::GET,
            "/v1/agent/self",
            Some(&c.device_secret),
            None::<&()>,
        )
        .await
    }
}
/// Production is fixed; alternate endpoints must be explicit loopback development origins.
pub(crate) fn validate_server(server: &str) -> Result<String> {
    let url = reqwest::Url::parse(server)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Agent server must be an origin without credentials or a path"
    );
    let origin = url.origin().ascii_serialization();
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(
        origin == PRODUCTION_API || (loopback && matches!(url.scheme(), "http" | "https")),
        "Agent server must be the production API or an explicit loopback development origin"
    );
    Ok(origin)
}
pub(crate) fn fresh_credentials(server: String) -> Result<Credentials> {
    let enrollment_secret = new_secret("enroll")?;
    let device_secret = new_secret("device")?;
    Ok(Credentials {
        server,
        request: EnrollmentRequest {
            request_id: RequestId::new(),
            enrollment_digest: secret_digest("enroll", &enrollment_secret)?,
            device_digest: secret_digest("device", &device_secret)?,
            metadata: DeviceMetadata {
                name: "My computer".into(),
                os: std::env::consts::OS.into(),
                architecture: std::env::consts::ARCH.into(),
                agent_version: env!("CARGO_PKG_VERSION").into(),
                protocol_version: PROTOCOL_VERSION,
            },
        },
        enrollment_secret,
        device_secret,
        started: None,
        device_id: None,
    })
}
/// One sequential network loop; cancel by dropping the future. The caller releases its lease.
pub(crate) async fn presence(
    client: &Client,
    c: &Credentials,
    lease_out: &mut Option<Lease>,
    mut report: impl FnMut(&str),
) -> Result<()> {
    let instance_id = InstanceId::new();
    let mut attempt = 0;
    loop {
        let response: std::result::Result<Lease, AgentError> = match lease_out.as_ref() {
            Some(lease) => {
                client
                    .request(
                        reqwest::Method::POST,
                        "/v1/agent/heartbeat",
                        Some(&c.device_secret),
                        Some(&LeaseRequest {
                            lease_id: lease.lease_id,
                        }),
                    )
                    .await
            }
            None => {
                client
                    .request(
                        reqwest::Method::POST,
                        "/v1/agent/connect",
                        Some(&c.device_secret),
                        Some(&ConnectRequest {
                            instance_id,
                            protocol_version: PROTOCOL_VERSION,
                        }),
                    )
                    .await
            }
        };
        let delay = match response {
            Ok(lease) => {
                *lease_out = Some(lease);
                attempt = 0;
                report("Online (presence only; no tools enabled)");
                HEARTBEAT_SECONDS + jitter() % 4
            }
            Err(e) if e.code == DeviceError::StaleLease => {
                *lease_out = None;
                let delay = retry_seconds(attempt, jitter() as u8);
                attempt = attempt.saturating_add(1);
                delay
            }
            Err(e) if e.retryable() => {
                report("Server unreachable or busy; reconnecting with bounded backoff");
                let delay = retry_seconds(attempt, jitter() as u8).max(e.retry_after);
                attempt = attempt.saturating_add(1);
                delay
            }
            Err(e) => return Err(e.into()),
        };
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
    }
}
fn jitter() -> u64 {
    let mut bytes = [0u8; 1];
    let _ = getrandom::getrandom(&mut bytes);
    u64::from(bytes[0])
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn origin_restrictions() {
        assert_eq!(validate_server(PRODUCTION_API).unwrap(), PRODUCTION_API);
        assert!(validate_server("http://127.0.0.1:7777").is_ok());
        for s in [
            "https://evil.example",
            "http://hosted-api.windieos.com",
            "https://hosted-api.windieos.com/path",
            "https://user@hosted-api.windieos.com",
            "https://hosted-api.windieos.com?x=y",
        ] {
            assert!(validate_server(s).is_err(), "{s}");
        }
    }
    #[test]
    fn auth_is_not_retryable() {
        assert!(
            !AgentError {
                code: DeviceError::Unauthorized,
                retry_after: 0
            }
            .retryable()
        );
    }
}
