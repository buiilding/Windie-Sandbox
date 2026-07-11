//! Bifrost model discovery and parameter metadata tests.

use super::*;
use crate::llm::ModelParameterOption;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use std::collections::HashMap;
use tokio::net::TcpListener;

#[test]
fn builds_models_endpoint_from_base_url() {
    let base_url = BaseUrl::new("http://localhost:8080/v1/");

    assert_eq!(
        models_endpoint(&base_url),
        "http://localhost:8080/v1/models"
    );
}

#[test]
fn builds_model_parameters_endpoint_from_base_url() {
    let base_url = BaseUrl::new("http://localhost:8080/v1/");

    assert_eq!(
        model_parameters_endpoint(&base_url, "openai/gpt-5.5")
            .unwrap()
            .as_str(),
        "http://localhost:8080/api/models/parameters?model=openai%2Fgpt-5.5"
    );
}

#[test]
fn builds_model_parameter_lookup_names_from_most_specific_to_least_specific() {
    assert_eq!(
        model_parameter_lookup_names("openrouter/moonshotai/kimi-k2.5"),
        vec![
            "openrouter/moonshotai/kimi-k2.5",
            "moonshotai/kimi-k2.5",
            "kimi-k2.5"
        ]
    );
    assert_eq!(
        model_parameter_lookup_names("anthropic/claude-fable-5"),
        vec!["anthropic/claude-fable-5", "claude-fable-5"]
    );
    assert_eq!(
        model_parameter_lookup_names("local-model"),
        vec!["local-model"]
    );
}

#[tokio::test]
async fn model_parameter_lookup_uses_first_successful_identity() {
    let base_url = model_parameter_test_server().await;

    let parameters = model_parameters(base_url, &ModelName::new("openrouter/moonshotai/kimi-k2.5"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(parameters.supports_reasoning, Some(true));
    assert_eq!(parameters.raw["matched_model"], "kimi-k2.5");
}

#[tokio::test]
async fn model_parameter_lookup_returns_none_when_all_identities_are_missing() {
    let base_url = model_parameter_test_server().await;

    let parameters = model_parameters(
        base_url,
        &ModelName::new("openrouter/example/missing-model"),
    )
    .await
    .unwrap();

    assert!(parameters.is_none());
}

async fn model_parameter_test_server() -> BaseUrl {
    async fn parameters(
        Query(query): Query<HashMap<String, String>>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let model = query.get("model").map(String::as_str).unwrap_or_default();
        if model == "kimi-k2.5" {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "matched_model": model,
                    "supports_reasoning": true,
                    "model_parameters": []
                })),
            );
        }

        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"missing"})),
        )
    }

    let app = Router::new().route("/api/models/parameters", get(parameters));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    BaseUrl::new(format!("http://{address}/v1"))
}

#[test]
fn decodes_model_parameter_options() {
    let raw = serde_json::json!({
        "supports_reasoning": true,
        "model_parameters": [{
            "id": "reasoning_effort",
            "type": "select",
            "label": "Reasoning Effort",
            "options": [
                {"label": "Low", "value": "low"},
                {"label": "High", "value": "high"}
            ]
        }]
    });

    let parameters = serde_json::from_value::<ModelParameterInfo>(raw).unwrap();

    assert_eq!(parameters.supports_reasoning, Some(true));
    assert_eq!(parameters.model_parameters[0].id, "reasoning_effort");
    assert_eq!(
        parameters.model_parameters[0].options,
        vec![
            ModelParameterOption {
                label: "Low".to_string(),
                value: "low".to_string(),
            },
            ModelParameterOption {
                label: "High".to_string(),
                value: "high".to_string(),
            },
        ]
    );
}
