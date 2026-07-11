//! Bifrost HTTP client tests.

use super::*;

#[test]
fn builds_responses_endpoint_from_base_url() {
    let llm = BifrostClient::new(
        BaseUrl::new("http://localhost:8080/v1/"),
        ModelName::new("openai/gpt-4o-mini"),
    );

    assert_eq!(
        llm.responses_endpoint(),
        "http://localhost:8080/v1/responses"
    );
}

#[test]
fn builds_input_tokens_endpoint_from_base_url() {
    let llm = BifrostClient::new(
        BaseUrl::new("http://localhost:8080/v1/"),
        ModelName::new("openai/gpt-4o-mini"),
    );

    assert_eq!(
        llm.input_tokens_endpoint(),
        "http://localhost:8080/v1/responses/input_tokens"
    );
}

#[test]
fn parses_input_token_count_response() {
    let response = input_token_count_from_raw(serde_json::json!({
        "object": "response.input_tokens",
        "model": "openai/gpt-4o-mini",
        "input_tokens": 42,
        "total_tokens": 45,
        "input_tokens_details": {"cached_tokens": 3}
    }))
    .unwrap();

    assert_eq!(response.input_tokens, 42);
    assert_eq!(response.total_tokens, Some(45));
    assert_eq!(response.model.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(response.raw["input_tokens_details"]["cached_tokens"], 3);
}

#[test]
fn recognizes_unsupported_input_token_count_response() {
    let body = r#"{"error":{"code":"unsupported_operation","message":"count_tokens is not supported by openrouter provider"},"extra_fields":{"request_type":"count_tokens"}}"#;

    assert!(is_unsupported_input_token_count_response(body));
    assert!(!is_unsupported_input_token_count_response(
        r#"{"error":{"code":"internal_error","message":"temporary failure"}}"#
    ));
    assert!(!is_unsupported_input_token_count_response(
        "500 Internal Server Error"
    ));
}
