//! Bounded retry policy for one model turn.
//!
//! A retry stays on the same conversation head and repeats the same model
//! request. No assistant message or tool call is persisted until the stream
//! completes successfully, so a failed attempt can be discarded safely.

use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::time::sleep;

use crate::llm::{AssistantResponse, LlmError, LlmStreamEvent, RuntimeLlm};
use crate::output::RuntimeOutput;
use crate::runtime::context::ModelContext;

use super::RuntimeModelRequest;

const MAX_MODEL_ATTEMPTS: u8 = 3;
const BASE_RETRY_DELAY: Duration = Duration::from_millis(500);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(8);

/// Streams one assistant turn, retrying only classified transient failures.
pub(crate) async fn stream_with_retry<O, L>(
    output: &O,
    llm: &L,
    context: &ModelContext,
    request: RuntimeModelRequest<'_>,
) -> Result<AssistantResponse>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    let mut last_error = None;

    for attempt in 0..MAX_MODEL_ATTEMPTS {
        output.start_assistant_message();
        let result = llm
            .stream(
                &context.messages,
                &context.tool_schemas,
                request.reasoning,
                request.prompt_cache,
                |event| match event {
                    LlmStreamEvent::AssistantDelta(text) => output.assistant_delta(text),
                    LlmStreamEvent::ReasoningDelta(text) => output.reasoning_delta(text),
                    LlmStreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta,
                    } => output.tool_call_delta(index, id, name, arguments_delta),
                },
            )
            .await;

        match result {
            Ok(response) => {
                output.end_assistant_message();
                return Ok(response);
            }
            Err(error) => {
                // Failed attempts are never allowed to remain in a client's
                // transient preview, even when the error is terminal.
                output.assistant_attempt_reset();

                let Some(llm_error) = find_llm_error(&error) else {
                    return Err(error);
                };
                if !llm_error.kind().is_retryable() || attempt + 1 >= MAX_MODEL_ATTEMPTS {
                    return Err(error);
                }

                let delay = retry_delay(llm_error, attempt);
                last_error = Some(error);
                sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("model retry loop ended without an attempt")))
}

/// Finds the typed provider failure through any contextual anyhow wrappers.
fn find_llm_error(error: &anyhow::Error) -> Option<&LlmError> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<LlmError>())
}

/// Chooses a bounded exponential delay, preferring the provider's guidance.
fn retry_delay(error: &LlmError, attempt: u8) -> Duration {
    error
        .retry_after()
        .unwrap_or_else(|| {
            let multiplier = 2u32.saturating_pow(u32::from(attempt));
            BASE_RETRY_DELAY
                .checked_mul(multiplier)
                .unwrap_or(MAX_RETRY_DELAY)
        })
        .min(MAX_RETRY_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmErrorKind;

    #[test]
    fn retry_delay_grows_and_is_bounded() {
        let error = LlmError::new(LlmErrorKind::ProviderOverloaded, "busy");

        assert_eq!(retry_delay(&error, 0), Duration::from_millis(500));
        assert_eq!(retry_delay(&error, 1), Duration::from_secs(1));
        assert_eq!(retry_delay(&error, 8), MAX_RETRY_DELAY);
    }

    #[test]
    fn provider_retry_after_wins_over_local_backoff() {
        let error = LlmError::new(LlmErrorKind::RateLimited, "limited")
            .with_retry_after(Some(Duration::from_secs(12)));

        assert_eq!(retry_delay(&error, 0), MAX_RETRY_DELAY);
    }
}
