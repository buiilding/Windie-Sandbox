//! Persistent MCP session lifecycle.
//!
//! This module owns provider-keyed session reuse, idle cleanup, and the
//! transport-neutral active session wrapper.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};

use super::http;
use super::protocol::{McpCommand, McpTransport};
use super::stdio;

const MCP_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MCP_IDLE_REAPER_INTERVAL: Duration = Duration::from_secs(30);

/// Owns persistent MCP provider sessions for one registry/client.
///
/// The persistent session is keyed by provider ID, not command string, because
/// provider identity is the routing boundary used by attached tool schemas. The
/// session is stopped after a period of inactivity, and stopping the session
/// also runs the provider shutdown hook when one is configured.
#[derive(Clone)]
pub struct McpSessionPool {
    sessions:
        Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<PersistentMcpSession>>>>>,
}

impl std::fmt::Debug for McpSessionPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpSessionPool")
            .finish_non_exhaustive()
    }
}

impl McpSessionPool {
    /// Creates a registry-owned persistent MCP session pool.
    pub fn new() -> Self {
        let sessions = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        spawn_idle_reaper(Arc::downgrade(&sessions));

        Self { sessions }
    }

    /// Calls one MCP provider tool through this pool's persistent session.
    pub async fn call_tool(
        &self,
        provider_id: &str,
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
        name: &str,
        arguments: Value,
    ) -> Result<Value> {
        self.call_tool_with_transport(
            provider_id,
            McpTransport::Stdio {
                command,
                shutdown_command,
            },
            name,
            arguments,
        )
        .await
    }

    /// Calls one tool through a persistent session for any supported MCP
    /// transport.
    pub async fn call_tool_with_transport(
        &self,
        provider_id: &str,
        transport: McpTransport,
        name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let session = self.ensure_session(provider_id, transport).await?;
        let result = {
            let mut session = session.lock().await;
            session.last_used_at = Instant::now();
            session
                .session
                .call(
                    "tools/call",
                    Some(json!({
                        "name": name,
                        "arguments": arguments
                    })),
                )
                .await
        };
        if result.is_err() {
            self.stop_session(provider_id, &session).await;
        }
        result
    }

    /// Stops and forgets one provider's persistent session before its runtime
    /// is removed from disk.
    pub async fn stop_provider(&self, provider_id: &str) {
        let session = self.sessions.lock().await.remove(provider_id);
        if let Some(session) = session {
            let mut session = session.lock().await;
            session.shutdown().await;
        }
    }

    /// Returns a matching persistent session, creating it when necessary.
    async fn ensure_session(
        &self,
        provider_id: &str,
        transport: McpTransport,
    ) -> Result<Arc<tokio::sync::Mutex<PersistentMcpSession>>> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(provider_id).cloned() {
            if session.lock().await.transport == transport {
                return Ok(session);
            }
            sessions.remove(provider_id);
            drop(sessions);
            let mut session = session.lock().await;
            session.shutdown().await;
            sessions = self.sessions.lock().await;
        }

        let session = Arc::new(tokio::sync::Mutex::new(PersistentMcpSession {
            transport: transport.clone(),
            session: McpTransportSession::start(transport).await?,
            last_used_at: Instant::now(),
        }));
        sessions.insert(provider_id.to_string(), Arc::clone(&session));
        Ok(session)
    }

    /// Stops one persistent provider session if it is still the active entry.
    async fn stop_session(
        &self,
        provider_id: &str,
        expected: &Arc<tokio::sync::Mutex<PersistentMcpSession>>,
    ) {
        let removed = {
            let mut sessions = self.sessions.lock().await;
            if sessions
                .get(provider_id)
                .is_some_and(|session| Arc::ptr_eq(session, expected))
            {
                sessions.remove(provider_id)
            } else {
                None
            }
        };
        if let Some(session) = removed {
            let mut session = session.lock().await;
            session.shutdown().await;
        }
    }
}

impl Default for McpSessionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Starts a small background task that stops idle persistent MCP sessions.
fn spawn_idle_reaper(
    sessions: std::sync::Weak<
        tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<PersistentMcpSession>>>>,
    >,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        loop {
            tokio::time::sleep(MCP_IDLE_REAPER_INTERVAL).await;
            let Some(sessions) = sessions.upgrade() else {
                break;
            };
            let entries = sessions.lock().await.clone();
            for (provider_id, session) in entries {
                let idle = {
                    let session = session.lock().await;
                    Instant::now().duration_since(session.last_used_at) >= MCP_IDLE_TIMEOUT
                };
                if !idle {
                    continue;
                }
                let removed = {
                    let mut sessions = sessions.lock().await;
                    if sessions
                        .get(&provider_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &session))
                    {
                        sessions.remove(&provider_id)
                    } else {
                        None
                    }
                };
                if let Some(session) = removed {
                    let mut session = session.lock().await;
                    session.shutdown().await;
                }
            }
        }
    });
}
/// Runtime state for one persistent MCP provider.
struct PersistentMcpSession {
    transport: McpTransport,
    session: McpTransportSession,
    last_used_at: Instant,
}

impl PersistentMcpSession {
    /// Shuts down the provider transport while its per-provider lock is held.
    pub(super) async fn shutdown(&mut self) {
        self.session.shutdown().await;
    }
}

/// One active MCP session over a supported transport.
pub(super) enum McpTransportSession {
    /// Local process-backed MCP session.
    Stdio(stdio::McpSession),
    /// Hosted HTTP-backed MCP session.
    StreamableHttp(http::StreamableHttpSession),
}

impl McpTransportSession {
    /// Starts and initializes one transport-specific MCP session.
    pub(super) async fn start(transport: McpTransport) -> Result<Self> {
        match transport {
            McpTransport::Stdio { command, .. } => {
                Ok(Self::Stdio(stdio::McpSession::start(command)?))
            }
            McpTransport::PackagedStdio { command, .. } => {
                Ok(Self::Stdio(stdio::McpSession::start_owned(command)?))
            }
            McpTransport::StreamableHttp { endpoint } => Ok(Self::StreamableHttp(
                http::StreamableHttpSession::start(endpoint).await?,
            )),
        }
    }

    /// Sends one MCP request and returns its result.
    pub(super) async fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        match self {
            Self::Stdio(session) => session.call(method, params),
            Self::StreamableHttp(session) => session.call(method, params).await,
        }
    }

    /// Shuts down the active transport.
    pub(super) async fn shutdown(&mut self) {
        if let Self::StreamableHttp(session) = self {
            session.shutdown().await;
        }
    }
}
