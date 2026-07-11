//! Explicit live-provider benchmark request.

use super::{
    BENCH_PROMPT, BaseUrl, BifrostClient, Duration, Instant, LlmStreamEvent, Message, ModelName,
    Result, Role,
};

/// Sends the tiny live request and measures first-token and full-response
/// latency.
pub(super) async fn run_live_request(
    base_url: &BaseUrl,
    model: &ModelName,
) -> Result<(Option<Duration>, Duration, usize)> {
    let llm = BifrostClient::new(base_url.clone(), model.clone());
    let messages = vec![Message {
        id: None,
        parent_message_id: None,
        role: Role::User,
        content: BENCH_PROMPT.to_string(),
        parts: Vec::new(),
        metadata: None,
    }];

    let request_started = Instant::now();
    let mut first_token = None;
    let response = llm
        .stream(&messages, &[], None, None, |event| {
            let LlmStreamEvent::AssistantDelta(delta) = event else {
                return Ok(());
            };

            if first_token.is_none() && !delta.is_empty() {
                first_token = Some(request_started.elapsed());
            }

            Ok(())
        })
        .await?;
    let full_response = request_started.elapsed();

    Ok((first_token, full_response, response.content.len()))
}
