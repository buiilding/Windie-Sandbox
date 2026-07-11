//! LLM facade contract tests.

use super::*;

#[test]
fn base_url_removes_trailing_slash() {
    let base_url = BaseUrl::new("http://localhost:8080/v1/");

    assert_eq!(base_url.as_str(), "http://localhost:8080/v1");
}

#[test]
fn model_name_preserves_provider_prefix() {
    let model = ModelName::new("anthropic/claude-3-5-haiku");

    assert_eq!(model.as_str(), "anthropic/claude-3-5-haiku");
}
