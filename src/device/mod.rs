//! Shared enrollment and presence protocol. No tool or conversation authority.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const ENROLLMENT_SECONDS: i64 = 600;
pub const LEASE_SECONDS: i64 = 90;
pub const HEARTBEAT_SECONDS: u64 = 20;
pub const POLL_SECONDS: u64 = 3;
pub const REQUEST_SECONDS: u64 = 10;
pub const PRODUCTION_API: &str = "https://hosted-api.windieos.com";
pub const PAIRING_URL: &str = "https://app.windieos.com/devices/connect";

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
identifier!(DeviceId);
identifier!(EnrollmentId);
identifier!(RequestId);
identifier!(InstanceId);
identifier!(LeaseId);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceMetadata {
    pub name: String,
    pub os: String,
    pub architecture: String,
    pub agent_version: String,
    pub protocol_version: u32,
}
impl DeviceMetadata {
    pub fn validate(&self) -> Result<(), DeviceError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(DeviceError::UnsupportedProtocol);
        }
        if [
            &self.name,
            &self.os,
            &self.architecture,
            &self.agent_version,
        ]
        .iter()
        .any(|s| s.trim().is_empty() || s.len() > 120 || s.chars().any(char::is_control))
        {
            return Err(DeviceError::InvalidRequest);
        }
        Ok(())
    }
}

/// Only digests cross the public initiation endpoint. Never derive Debug for secrets.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRequest {
    pub request_id: RequestId,
    pub enrollment_digest: String,
    pub device_digest: String,
    pub metadata: DeviceMetadata,
}
impl EnrollmentRequest {
    pub fn validate(&self) -> Result<(), DeviceError> {
        self.metadata.validate()?;
        if !valid_digest(&self.enrollment_digest)
            || !valid_digest(&self.device_digest)
            || self.enrollment_digest == self.device_digest
        {
            return Err(DeviceError::InvalidRequest);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentStarted {
    pub id: EnrollmentId,
    pub code: String,
    pub verification_url: String,
    pub expires_at: i64,
    pub poll_interval_seconds: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentState {
    Pending,
    Approved,
    Consumed,
    Denied,
    Cancelled,
    Expired,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentView {
    pub id: EnrollmentId,
    pub state: EnrollmentState,
    pub metadata: DeviceMetadata,
    pub expires_at: i64,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub device_id: Option<DeviceId>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeRequest {
    pub code: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizeRequest {
    pub account_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceView {
    pub id: DeviceId,
    pub metadata: DeviceMetadata,
    pub revoked: bool,
    pub online: bool,
    pub last_seen: Option<i64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    pub instance_id: InstanceId,
    pub protocol_version: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseRequest {
    pub lease_id: LeaseId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub lease_id: LeaseId,
    pub expires_at: i64,
}

/// Safe public error codes: database/HTTP internals and credentials are never included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum DeviceError {
    #[error("Device request is invalid")]
    InvalidRequest,
    #[error("Credential invalid or revoked; explicitly reconnect the device")]
    Unauthorized,
    #[error("Registration not found")]
    NotFound,
    #[error("Enrollment expired; start a new enrollment")]
    Expired,
    #[error("Enrollment state or request conflicts")]
    Conflict,
    #[error("Another agent instance has an active lease")]
    AlreadyRunning,
    #[error("Presence lease expired or was superseded")]
    StaleLease,
    #[error("Unsupported agent protocol; update Windie")]
    UnsupportedProtocol,
    #[error("Too many requests; retry later")]
    RateLimited,
    #[error("Device registration is unavailable")]
    Unavailable,
}
impl From<sqlx::Error> for DeviceError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Cryptographically generated bearer, with a principal-specific prefix.
pub fn new_secret(kind: &str) -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| anyhow::anyhow!("OS random source unavailable"))?;
    Ok(format!("wd_{kind}_{}", hex(&bytes)))
}
pub fn secret_digest(kind: &str, secret: &str) -> Result<String, DeviceError> {
    let prefix = format!("wd_{kind}_");
    if !secret.strip_prefix(&prefix).is_some_and(valid_digest) {
        return Err(DeviceError::Unauthorized);
    }
    Ok(hash(&format!("windie:{kind}:v1:{secret}")))
}
pub fn hash(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))
}
pub fn keyed(key: &[u8], domain: &str, value: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(domain.as_bytes());
    mac.update(b"\0");
    mac.update(value.as_bytes());
    hex(&mac.finalize().into_bytes())
}
/// Twelve hex characters would only give 48 bits: use fourteen (56 bits).
pub fn enrollment_code(key: &[u8], id: EnrollmentId) -> String {
    let code = keyed(key, "enrollment-code-v1", &id.to_string());
    format!("{}-{}", &code[..7], &code[7..14]).to_uppercase()
}
pub fn normalize_code(code: &str) -> Result<String, DeviceError> {
    let normalized: String = code
        .chars()
        .filter(|c| *c != '-' && !c.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.len() != 14 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(DeviceError::NotFound);
    }
    Ok(normalized)
}
pub fn retry_seconds(attempt: u32, jitter: u8) -> u64 {
    (1u64 << attempt.min(4)) + u64::from(jitter % 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_principal_specific() {
        let a = new_secret("enroll").unwrap();
        let b = new_secret("device").unwrap();
        assert_ne!(a, b);
        assert!(secret_digest("device", &a).is_err());
        assert!(valid_digest(&secret_digest("device", &b).unwrap()));
        assert!(!format!("{:?}", DeviceError::Unauthorized).contains(&b));
    }
    #[test]
    fn codes_are_stable_normalized_and_key_bound() {
        let id = EnrollmentId::new();
        let code = enrollment_code(b"one", id);
        assert_eq!(code, enrollment_code(b"one", id));
        assert_ne!(code, enrollment_code(b"two", id));
        assert_eq!(normalize_code(&code).unwrap().len(), 14);
        assert!(normalize_code("bad").is_err());
    }
    #[test]
    fn retry_is_bounded() {
        for n in 0..100 {
            assert!((1..=30).contains(&retry_seconds(n, 255)));
        }
    }
}
