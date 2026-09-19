//! Outbound device agent transport. Tool opt-in reuses local plugin/MCP code
//! but never starts Bifrost, a local API, or a local hosted-chat session.

pub(crate) mod enrollment;
pub(crate) mod execution;
pub(crate) mod journal;
pub(crate) mod storage;
#[cfg(test)]
mod tests;
use crate::device::*;
use crate::{
    plugin::{PluginCatalog, PluginStore, bundled_index},
    store::Store,
    tool::ToolProviderRegistry,
};
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
    work_http: reqwest::Client,
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
            work_http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(WORK_REQUEST_SECONDS))
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
        self.request_with(&self.http, 65_536, method, path, secret, body)
            .await
    }

    async fn request_work<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        secret: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> std::result::Result<T, AgentError> {
        self.request_with(&self.work_http, 524_288, method, path, secret, body)
            .await
    }

    async fn request_with<T: DeserializeOwned>(
        &self,
        http: &reqwest::Client,
        maximum_response_bytes: usize,
        method: reqwest::Method,
        path: &str,
        secret: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> std::result::Result<T, AgentError> {
        let unavailable = || AgentError {
            code: DeviceError::Unavailable,
            retry_after: 0,
        };
        let mut request = http.request(method, format!("{}{}", self.server, path));
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
            if bytes.len() + chunk.len() > maximum_response_bytes {
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

    /// Builds a descriptive local capability report using the exact existing
    /// SQLite/plugin catalog path. This starts neither a local API nor Bifrost.
    pub fn capability_report(&self, lease_id: LeaseId) -> Result<CapabilityReport> {
        let store = Store::open()?;
        let plugins = std::sync::Arc::new(PluginStore::default_store()?);
        let catalog = PluginCatalog::new(plugins, bundled_index()?);
        let registry = ToolProviderRegistry::with_installed_plugins()?;
        let mut capabilities = catalog.build_capability_snapshot(&store, &registry)?;
        // Marketplace availability belongs to the hosted product catalog, not
        // this machine's execution report.  The agent reports only its
        // installed capability facts; the shared index renderer still handles
        // a valid empty installed list.
        capabilities.index.available.clear();
        Ok(CapabilityReport {
            version: PROTOCOL_VERSION,
            lease_id,
            capabilities,
        })
    }

    /// Reports installed capabilities through the device credential boundary.
    pub async fn publish_capabilities(
        &self,
        c: &Credentials,
        lease_id: LeaseId,
    ) -> std::result::Result<CapabilityAccepted, AgentError> {
        let report = self.capability_report(lease_id).map_err(|_| AgentError {
            code: DeviceError::Unavailable,
            retry_after: 0,
        })?;
        self.request_work(
            reqwest::Method::POST,
            "/v1/agent/capabilities",
            Some(&c.device_secret),
            Some(&report),
        )
        .await
    }

    /// Long-polls for one assignment. The server returns `null` for normal
    /// no-work responses; an agent never polls another device's queue.
    pub async fn next_work(
        &self,
        c: &Credentials,
        lease_id: LeaseId,
    ) -> std::result::Result<Option<DeviceWorkAssignment>, AgentError> {
        self.request_work(
            reqwest::Method::POST,
            "/v1/agent/work/next",
            Some(&c.device_secret),
            Some(&LeaseRequest { lease_id }),
        )
        .await
    }

    /// Fences execution after the journal has recorded delivery. Empty success
    /// bodies deserialize as unit, so this route cannot leak assignment state.
    pub async fn start_work(
        &self,
        c: &Credentials,
        assignment: &DeviceWorkAssignment,
    ) -> std::result::Result<(), AgentError> {
        self.request_work(
            reqwest::Method::POST,
            &format!("/v1/agent/work/{}/start", assignment.id),
            Some(&c.device_secret),
            Some(&DeviceWorkAuthorization {
                lease_id: assignment.lease_id,
                execution_token: assignment.execution_token.clone(),
            }),
        )
        .await
    }

    /// Re-sends the exact journaled result after a lost acknowledgement.
    pub async fn submit_work_result(
        &self,
        c: &Credentials,
        assignment: &DeviceWorkAssignment,
        result: &DeviceWorkResult,
    ) -> std::result::Result<DeviceWorkResultAccepted, AgentError> {
        self.request_work(
            reqwest::Method::POST,
            &format!("/v1/agent/work/{}/result", assignment.id),
            Some(&c.device_secret),
            Some(&DeviceWorkResultRequest {
                authorization: DeviceWorkAuthorization {
                    lease_id: assignment.lease_id,
                    execution_token: assignment.execution_token.clone(),
                },
                result: result.clone(),
            }),
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

/// Presence plus explicit capability publication and assignment polling. This
/// is intentionally a separate opt-in loop: `windie agent run` continues to
/// have exactly the old presence-only authority.
pub(crate) async fn presence_with_tools(
    client: &Client,
    c: &Credentials,
    storage: &storage::Storage,
    lease_out: &mut Option<Lease>,
    mut report: impl FnMut(&str),
) -> Result<()> {
    let instance_id = InstanceId::new();
    let mut attempt = 0u32;
    let mut recovered_journal = false;
    loop {
        let lease: std::result::Result<Lease, AgentError> = match lease_out.as_ref() {
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
        let lease = match lease {
            Ok(lease) => {
                *lease_out = Some(lease.clone());
                attempt = 0;
                lease
            }
            Err(error) if error.code == DeviceError::StaleLease => {
                *lease_out = None;
                tokio::time::sleep(std::time::Duration::from_secs(retry_seconds(
                    attempt,
                    jitter() as u8,
                )))
                .await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            Err(error) if error.retryable() => {
                report("Server unreachable or busy; reconnecting with bounded backoff");
                tokio::time::sleep(std::time::Duration::from_secs(
                    retry_seconds(attempt, jitter() as u8).max(error.retry_after),
                ))
                .await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        match client.publish_capabilities(c, lease.lease_id).await {
            Ok(_) => report("Online with explicitly enabled local plugin capabilities"),
            Err(error) if error.retryable() => {
                report("Could not publish capabilities; retrying without executing work");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        if !recovered_journal {
            recover_journaled_work(client, c, storage).await?;
            recovered_journal = true;
        }
        match client.next_work(c, lease.lease_id).await {
            Ok(Some(assignment)) => {
                match execute_assignment_while_heartbeating(
                    client,
                    c,
                    storage,
                    lease.lease_id,
                    assignment,
                )
                .await
                {
                    Ok(()) => report("Completed one approved hosted assignment"),
                    Err(error) => {
                        report("Assignment needs recovery; no automatic repeat will occur");
                        return Err(error);
                    }
                }
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_secs(WORK_POLL_SECONDS)).await,
            Err(error) if error.retryable() => {
                tokio::time::sleep(std::time::Duration::from_secs(
                    retry_seconds(attempt, jitter() as u8).max(error.retry_after),
                ))
                .await
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Recovers the one protected assignment journal before accepting new work.
/// A completed result can be sent again without repeating local side effects;
/// an execution that was interrupted is intentionally made uncertain instead.
async fn recover_journaled_work(
    client: &Client,
    credentials: &Credentials,
    storage: &storage::Storage,
) -> Result<()> {
    use journal::{WorkJournal, WorkJournalState};

    let journal = WorkJournal::new(storage);
    let Some(mut entry) = journal.load()? else {
        return Ok(());
    };
    match entry.state {
        WorkJournalState::Completed => {
            let result = entry
                .result
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Completed device journal has no result"))?;
            client
                .submit_work_result(credentials, &entry.assignment, result)
                .await?;
            Ok(())
        }
        WorkJournalState::Accepted => {
            execute_assignment(client, credentials, storage, entry.assignment).await
        }
        WorkJournalState::Executing => {
            journal.mark_uncertain(&mut entry)?;
            anyhow::bail!(
                "A prior tool execution was interrupted; refusing automatic re-execution"
            );
        }
        WorkJournalState::Uncertain => {
            anyhow::bail!("A prior tool execution is uncertain and needs operator recovery")
        }
    }
}

/// Keeps presence alive while the existing local registry or MCP executor is
/// running.  The assignment is still strictly sequential; the heartbeat has
/// no command payload and cannot start another task.
async fn execute_assignment_while_heartbeating(
    client: &Client,
    credentials: &Credentials,
    storage: &storage::Storage,
    lease_id: LeaseId,
    assignment: DeviceWorkAssignment,
) -> Result<()> {
    let worker_client = client.clone();
    let worker_credentials = credentials.clone();
    let worker_storage = storage.clone();
    let mut worker = tokio::spawn(async move {
        execute_assignment(
            &worker_client,
            &worker_credentials,
            &worker_storage,
            assignment,
        )
        .await
    });
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_SECONDS));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = &mut worker => {
                return result.map_err(|error| anyhow::anyhow!("device assignment task stopped: {error}"))?;
            }
            _ = heartbeat.tick() => {
                // Do not cancel local work on a transient heartbeat failure:
                // its journal/result protocol is the authority for recovery.
                let _ = client.request::<Lease>(
                    reqwest::Method::POST,
                    "/v1/agent/heartbeat",
                    Some(&credentials.device_secret),
                    Some(&LeaseRequest { lease_id }),
                ).await;
            }
        }
    }
}

/// Runs one delivery through the protected journal. The server first fences a
/// start; the local registry is entered only after that acknowledgement.
async fn execute_assignment(
    client: &Client,
    credentials: &Credentials,
    storage: &storage::Storage,
    assignment: DeviceWorkAssignment,
) -> Result<()> {
    use journal::{WorkJournal, WorkJournalState};

    let journal = WorkJournal::new(storage);
    let mut entry = journal.accept(assignment)?;
    match entry.state {
        WorkJournalState::Completed => {
            let result = entry
                .result
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Completed device journal has no result"))?;
            client
                .submit_work_result(credentials, &entry.assignment, result)
                .await?;
            return Ok(());
        }
        WorkJournalState::Executing => {
            journal.mark_uncertain(&mut entry)?;
            anyhow::bail!(
                "A prior tool execution was interrupted; refusing automatic re-execution"
            );
        }
        WorkJournalState::Uncertain => {
            anyhow::bail!("A prior tool execution is uncertain and needs operator recovery");
        }
        WorkJournalState::Accepted => {}
    }
    client.start_work(credentials, &entry.assignment).await?;
    journal.mark_executing(&mut entry)?;
    let result = execution::execute(&entry.assignment).await;
    journal.complete(&mut entry, result)?;
    client
        .submit_work_result(
            credentials,
            &entry.assignment,
            entry
                .result
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Device execution result was not journaled"))?,
        )
        .await?;
    Ok(())
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
