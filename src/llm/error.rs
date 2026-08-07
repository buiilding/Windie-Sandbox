//! Typed failures returned by the provider-facing LLM boundary.
//!
//! The LLM client preserves enough provider information for the runtime to
//! distinguish transient capacity failures from invalid requests. The runtime
//! owns retry policy; this module only classifies the failure and preserves the
//! display message needed for final diagnostics.

use std::error::Error;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Stable category for one provider/model request failure.
pub enum LlmErrorKind {
    /// The selected provider is temporarily at capacity.
    ProviderOverloaded,
    /// A request or token rate limit was reached.
    RateLimited,
    /// The provider endpoint was unavailable or returned a temporary server error.
    ProviderUnavailable,
    /// The provider or gateway timed out while handling the request.
    Timeout,
    /// The connection failed while sending or reading the request.
    Transport,
    /// The request cannot succeed without changing its contents.
    InvalidRequest,
    /// Provider credentials are missing, invalid, or revoked.
    Authentication,
    /// The account or key cannot pay for the request.
    PaymentRequired,
    /// The request exceeds the model's context or token limits.
    ContextLength,
    /// The provider does not support the requested operation or model.
    Unsupported,
    /// The failure is not safe to retry automatically.
    Unknown,
}

impl LlmErrorKind {
    /// Returns whether retrying the same request may succeed later.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::ProviderOverloaded
                | Self::RateLimited
                | Self::ProviderUnavailable
                | Self::Timeout
                | Self::Transport
        )
    }
}

#[derive(Debug)]
/// One classified provider failure with its user-facing diagnostic text.
pub struct LlmError {
    kind: LlmErrorKind,
    message: String,
    retry_after: Option<Duration>,
}

impl LlmError {
    /// Creates a typed failure with no provider-supplied retry delay.
    pub fn new(kind: LlmErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: None,
        }
    }

    /// Classifies a provider error using its canonical type, native code,
    /// message, and optional HTTP status.
    pub fn from_provider_fields(
        error_type: Option<&str>,
        code: Option<&str>,
        message: impl Into<String>,
        status: Option<u16>,
    ) -> Self {
        let message = message.into();
        let kind = classify_provider_failure(error_type, code, &message, status);
        Self::new(kind, message)
    }

    /// Classifies one non-success HTTP response while preserving its complete
    /// status/body display text for the final session failure.
    pub fn from_http_response(status: u16, body: &str) -> Self {
        let raw = serde_json::from_str::<serde_json::Value>(body).ok();
        let error = raw.as_ref().and_then(|value| value.get("error"));
        let error_type = raw
            .as_ref()
            .and_then(|value| value.get("error_type"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                error
                    .and_then(|value| value.get("metadata"))
                    .and_then(|value| value.get("error_type"))
                    .and_then(serde_json::Value::as_str)
            });
        let code = error
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str);

        Self::from_provider_fields(
            error_type,
            code,
            format!("responses request failed with {status}: {body}"),
            Some(status),
        )
    }

    /// Creates a transport failure and distinguishes timeouts from other I/O
    /// failures so the runtime can report the correct retry category.
    pub fn transport(message: impl Into<String>, timed_out: bool) -> Self {
        Self::new(
            if timed_out {
                LlmErrorKind::Timeout
            } else {
                LlmErrorKind::Transport
            },
            message,
        )
    }

    /// Returns the classified failure category.
    pub fn kind(&self) -> LlmErrorKind {
        self.kind
    }

    /// Returns the provider or transport diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns a provider-requested delay before retrying, when one exists.
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// Attaches a parsed `Retry-After` delay to this failure.
    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for LlmError {}

/// Classifies OpenRouter/Bifrost provider fields without relying on a single
/// wire format. The message fallback is needed for upstream-native errors such
/// as NVIDIA's `ResourceExhausted` response.
fn classify_provider_failure(
    error_type: Option<&str>,
    code: Option<&str>,
    message: &str,
    status: Option<u16>,
) -> LlmErrorKind {
    let error_type = error_type.unwrap_or_default().to_ascii_lowercase();
    let code = code.unwrap_or_default().to_ascii_lowercase();
    let message = message.to_ascii_lowercase();

    if matches!(
        error_type.as_str(),
        "authentication" | "authentication_error" | "invalid_api_key"
    ) || matches!(code.as_str(), "401" | "invalid_api_key")
    {
        return LlmErrorKind::Authentication;
    }
    if matches!(
        error_type.as_str(),
        "payment_required" | "insufficient_credits"
    ) || code == "402"
    {
        return LlmErrorKind::PaymentRequired;
    }
    if matches!(
        error_type.as_str(),
        "context_length_exceeded" | "max_tokens_exceeded" | "token_limit_exceeded"
    ) || message.contains("context length")
    {
        return LlmErrorKind::ContextLength;
    }
    if matches!(
        error_type.as_str(),
        "invalid_request" | "invalid_prompt" | "content_policy_violation"
    ) || matches!(code.as_str(), "400" | "invalid_prompt" | "invalid_request")
    {
        return LlmErrorKind::InvalidRequest;
    }
    if matches!(error_type.as_str(), "unsupported_operation" | "not_found")
        || code == "unsupported_operation"
    {
        return LlmErrorKind::Unsupported;
    }
    if matches!(error_type.as_str(), "rate_limit_exceeded" | "rate_limited")
        || matches!(
            code.as_str(),
            "429" | "rate_limit_exceeded" | "rate_limited"
        )
        || message.contains("rate limit")
        || message.contains("too many requests")
    {
        return LlmErrorKind::RateLimited;
    }
    if matches!(error_type.as_str(), "provider_overloaded" | "overloaded")
        || message.contains("resourceexhausted")
        || message.contains("worker local total request limit")
        || message.contains("provider overloaded")
        || message.contains("capacity")
    {
        return LlmErrorKind::ProviderOverloaded;
    }
    if matches!(error_type.as_str(), "timeout") || matches!(code.as_str(), "408" | "504") {
        return LlmErrorKind::Timeout;
    }
    if matches!(
        error_type.as_str(),
        "provider_unavailable" | "server" | "server_error"
    ) || matches!(status, Some(500..=599))
    {
        return LlmErrorKind::ProviderUnavailable;
    }

    LlmErrorKind::Unknown
}
