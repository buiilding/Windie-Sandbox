//! Tests for the Bifrost client boundary.

use super::model::{model_parameter_candidates, model_parameters_endpoint, models_endpoint};
use super::responses::{ResponsesInputTokensRequest, ResponsesRequest};
use super::serialization::{
    ImageInputDetail, image_input_detail_for_model, prompt_cache_fields, responses_input,
    responses_tools,
};
use super::stream::{
    AssistantStreamState, append_valid_utf8, finish_utf8, input_token_count_from_raw,
    process_stream_line, process_stream_lines,
};
use super::*;
use crate::conversation::assistant_metadata::{ToolCallFunction, ToolCallKind};
use crate::conversation::{
    ImageAssetId, ImagePart, MessageId, MessageMetadata, MessagePart, Role, ToolCall, ToolCallId,
};
use crate::tool::{ToolSchema, ToolSchemaName};

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

#[test]
fn selects_original_image_detail_for_supported_openai_models() {
    assert_eq!(
        image_input_detail_for_model("openai/gpt-5.4"),
        ImageInputDetail::Original
    );
    assert_eq!(
        image_input_detail_for_model("openai/gpt-5.5"),
        ImageInputDetail::Original
    );
    assert_eq!(
        image_input_detail_for_model("openai/gpt-5.6"),
        ImageInputDetail::Original
    );
}

#[test]
fn selects_high_image_detail_for_other_models() {
    assert_eq!(
        image_input_detail_for_model("openai/gpt-4o-mini"),
        ImageInputDetail::High
    );
    assert_eq!(
        image_input_detail_for_model("openai/gpt-5.4-mini"),
        ImageInputDetail::High
    );
    assert_eq!(
        image_input_detail_for_model("anthropic/claude-opus-4-7"),
        ImageInputDetail::High
    );
}

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
fn builds_model_parameter_candidates_for_nested_model_ids() {
    assert_eq!(
        model_parameter_candidates("openrouter/openai/gpt-4o-mini"),
        vec![
            "openrouter/openai/gpt-4o-mini",
            "openai/gpt-4o-mini",
            "gpt-4o-mini"
        ]
    );
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

#[test]
fn serializes_text_message_for_responses_request() {
    let messages = vec![Message {
        id: Some(MessageId::new("message-id")),
        parent_message_id: Some(MessageId::new("parent-id")),
        role: Role::User,
        content: "hello".to_string(),
        parts: Vec::new(),
        metadata: None,
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    let value = serde_json::to_value(request).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [{"type": "message", "role": "user", "content": "hello"}],
            "stream": true
        })
    );
}

#[test]
fn serializes_reasoning_effort_for_responses_request() {
    let reasoning = ReasoningRequest {
        effort: Some("high".to_string()),
        summary: None,
    };
    let request = ResponsesRequest {
        model: "openai/gpt-5.5",
        input: Vec::new(),
        tools: None,
        reasoning: Some(&reasoning),
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({
            "model": "openai/gpt-5.5",
            "input": [],
            "reasoning": {"effort": "high"},
            "stream": true
        })
    );
}

#[test]
fn serializes_reasoning_effort_and_summary_for_responses_request() {
    let reasoning = ReasoningRequest {
        effort: Some("high".to_string()),
        summary: Some("auto".to_string()),
    };
    let request = ResponsesRequest {
        model: "openai/gpt-5.5",
        input: Vec::new(),
        tools: None,
        reasoning: Some(&reasoning),
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({
            "model": "openai/gpt-5.5",
            "input": [],
            "reasoning": {"effort": "high", "summary": "auto"},
            "stream": true
        })
    );
}

#[test]
fn serializes_openai_prompt_cache_for_responses_request() {
    let prompt_cache = PromptCacheRequest {
        key: "windie:conversation-id".to_string(),
        retention: Some("24h".to_string()),
    };
    let prompt_cache_fields = prompt_cache_fields("openai/gpt-5.5", Some(&prompt_cache));
    let request = ResponsesRequest {
        model: "openai/gpt-5.5",
        input: Vec::new(),
        tools: None,
        reasoning: None,
        prompt_cache_key: prompt_cache_fields.prompt_cache_key,
        prompt_cache_retention: prompt_cache_fields.prompt_cache_retention,
        cache_control: prompt_cache_fields.cache_control,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({
            "model": "openai/gpt-5.5",
            "input": [],
            "prompt_cache_key": "windie:conversation-id",
            "prompt_cache_retention": "24h",
            "stream": true
        })
    );
}

#[test]
fn serializes_anthropic_prompt_cache_for_responses_request() {
    let prompt_cache = PromptCacheRequest {
        key: "windie:conversation-id".to_string(),
        retention: Some("24h".to_string()),
    };
    let prompt_cache_fields = prompt_cache_fields("anthropic/claude-opus-4-8", Some(&prompt_cache));
    let request = ResponsesRequest {
        model: "anthropic/claude-opus-4-8",
        input: Vec::new(),
        tools: None,
        reasoning: None,
        prompt_cache_key: prompt_cache_fields.prompt_cache_key,
        prompt_cache_retention: prompt_cache_fields.prompt_cache_retention,
        cache_control: prompt_cache_fields.cache_control,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({
            "model": "anthropic/claude-opus-4-8",
            "input": [],
            "cache_control": {"type": "ephemeral"},
            "stream": true
        })
    );
}

#[test]
fn omits_prompt_cache_for_unsupported_provider() {
    let prompt_cache = PromptCacheRequest {
        key: "windie:conversation-id".to_string(),
        retention: Some("24h".to_string()),
    };
    let prompt_cache_fields = prompt_cache_fields("groq/llama", Some(&prompt_cache));
    let request = ResponsesRequest {
        model: "groq/llama",
        input: Vec::new(),
        tools: None,
        reasoning: None,
        prompt_cache_key: prompt_cache_fields.prompt_cache_key,
        prompt_cache_retention: prompt_cache_fields.prompt_cache_retention,
        cache_control: prompt_cache_fields.cache_control,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({
            "model": "groq/llama",
            "input": [],
            "stream": true
        })
    );
}

#[test]
fn serializes_assistant_tool_calls_for_responses_request() {
    let messages = vec![Message {
        id: Some(MessageId::new("message-id")),
        parent_message_id: Some(MessageId::new("parent-id")),
        role: Role::Assistant,
        content: String::new(),
        parts: Vec::new(),
        metadata: Some(MessageMetadata {
            tool_calls: vec![ToolCall {
                index: 0,
                id: ToolCallId::new("call-id"),
                kind: ToolCallKind::Function,
                function: ToolCallFunction {
                    name: "run_shell".to_string(),
                    arguments: r#"{"command":"ls"}"#.to_string(),
                },
            }],
            ..Default::default()
        }),
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    let value = serde_json::to_value(request).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [{
                "type": "function_call",
                "call_id": "call-id",
                "name": "run_shell",
                "arguments": "{\"command\":\"ls\"}",
                "status": "completed"
            }],
            "stream": true
        })
    );
}

#[test]
fn serializes_assistant_text_before_tool_call_for_responses_request() {
    let messages = vec![Message {
        id: Some(MessageId::new("message-id")),
        parent_message_id: Some(MessageId::new("parent-id")),
        role: Role::Assistant,
        content: "I will inspect the desktop.".to_string(),
        parts: Vec::new(),
        metadata: Some(MessageMetadata {
            tool_calls: vec![ToolCall {
                index: 0,
                id: ToolCallId::new("call-id"),
                kind: ToolCallKind::Function,
                function: ToolCallFunction {
                    name: "cua_driver__get_desktop_state".to_string(),
                    arguments: "{}".to_string(),
                },
            }],
            ..Default::default()
        }),
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    let value = serde_json::to_value(request).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "I will inspect the desktop."
                    }]
                },
                {
                    "type": "function_call",
                    "call_id": "call-id",
                    "name": "cua_driver__get_desktop_state",
                    "arguments": "{}",
                    "status": "completed"
                }
            ],
            "stream": true
        })
    );
}

#[test]
fn serializes_user_image_parts_for_responses_request() {
    let messages = vec![Message {
        id: Some(MessageId::new("message-id")),
        parent_message_id: None,
        role: Role::User,
        content: "what is this?".to_string(),
        parts: vec![
            MessagePart::Text("what is this?".to_string()),
            MessagePart::Image(ImagePart {
                asset_id: ImageAssetId::new("image-id"),
                mime_type: "image/png".to_string(),
                bytes: vec![1, 2, 3],
            }),
        ],
        metadata: None,
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    let value = serde_json::to_value(request).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "what is this?"},
                    {"type": "input_image", "image_url": "data:image/png;base64,AQID", "detail": "high"}
                ]
            }],
            "stream": true
        })
    );
}

#[test]
fn serializes_tool_message_call_id_for_responses_request() {
    let messages = vec![Message {
        id: Some(MessageId::new("message-id")),
        parent_message_id: Some(MessageId::new("parent-id")),
        role: Role::Tool,
        content: r#"{"stdout":"ok"}"#.to_string(),
        parts: Vec::new(),
        metadata: Some(MessageMetadata {
            tool_call_id: Some(ToolCallId::new("call-id")),
            ..Default::default()
        }),
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [{
                "type": "function_call_output",
                "call_id": "call-id",
                "output": "{\"stdout\":\"ok\"}",
                "status": "completed"
            }],
            "stream": true
        })
    );
}

#[test]
fn serializes_tool_image_parts_for_responses_request() {
    let messages = vec![Message {
        id: Some(MessageId::new("message-id")),
        parent_message_id: Some(MessageId::new("parent-id")),
        role: Role::Tool,
        content: "screenshot".to_string(),
        parts: vec![
            MessagePart::Text("screenshot".to_string()),
            MessagePart::Image(ImagePart {
                asset_id: ImageAssetId::new("image-id"),
                mime_type: "image/png".to_string(),
                bytes: vec![1, 2, 3],
            }),
        ],
        metadata: Some(MessageMetadata {
            tool_call_id: Some(ToolCallId::new("call-id")),
            ..Default::default()
        }),
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [{
                "type": "function_call_output",
                "call_id": "call-id",
                "output": [
                    {"type": "input_text", "text": "screenshot"},
                    {"type": "input_image", "image_url": "data:image/png;base64,AQID", "detail": "high"}
                ],
                "status": "completed"
            }],
            "stream": true
        })
    );
}

#[test]
fn serializes_tool_schemas_for_responses_request() {
    let tools = vec![ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"}
            },
            "required": ["command"]
        }),
    }];
    let request = ResponsesRequest {
        model: "openai/gpt-4o-mini",
        input: Vec::new(),
        tools: responses_tools(&tools),
        reasoning: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        cache_control: None,
        stream: true,
    };

    let value = serde_json::to_value(request).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [],
            "tools": [{
                "type": "function",
                "name": "run_shell",
                "description": "Run a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    },
                    "required": ["command"]
                }
            }],
            "stream": true
        })
    );
}

#[test]
fn serializes_input_token_request_without_stream() {
    let messages = vec![Message {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content: "You are concise.".to_string(),
        parts: Vec::new(),
        metadata: None,
    }];
    let request = ResponsesInputTokensRequest {
        model: "openai/gpt-4o-mini",
        input: responses_input(&messages, ImageInputDetail::High),
        tools: None,
    };

    let value = serde_json::to_value(request).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "model": "openai/gpt-4o-mini",
            "input": [{"type": "message", "role": "system", "content": "You are concise."}]
        })
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
fn parses_stream_content_delta() {
    let mut state = AssistantStreamState::default();
    let mut deltas = Vec::new();
    let mut handle_delta = |event: LlmStreamEvent<'_>| -> Result<()> {
        if let LlmStreamEvent::AssistantDelta(text) = event {
            deltas.push(text.to_string());
        }
        Ok(())
    };

    process_stream_line(
        r#"data: {"type":"response.output_text.delta","delta":"Hello"}"#,
        &mut state,
        &mut handle_delta,
    )
    .unwrap();

    assert_eq!(state.content, "Hello");
    assert_eq!(deltas, vec!["Hello"]);
}

#[test]
fn parses_stream_metadata_lanes() {
    let mut state = AssistantStreamState::default();
    let mut reasoning_deltas = Vec::new();
    let mut handle_delta = |event: LlmStreamEvent<'_>| -> Result<()> {
        if let LlmStreamEvent::ReasoningDelta(text) = event {
            reasoning_deltas.push(text.to_string());
        }
        Ok(())
    };

    process_stream_line(
        r#"data: {"type":"response.refusal.delta","refusal":"no"}"#,
        &mut state,
        &mut handle_delta,
    )
    .unwrap();
    process_stream_line(
        r#"data: {"type":"response.reasoning_summary_text.delta","delta":"think"}"#,
        &mut state,
        &mut handle_delta,
    )
    .unwrap();

    let response = state.finalize().unwrap();

    assert_eq!(response.metadata.refusal.as_deref(), Some("no"));
    assert_eq!(response.metadata.reasoning.as_deref(), Some("think"));
    assert_eq!(reasoning_deltas, vec!["think"]);
}

#[test]
fn parses_reasoning_text_stream_metadata() {
    let mut state = AssistantStreamState::default();
    let mut reasoning_deltas = Vec::new();
    let mut handle_delta = |event: LlmStreamEvent<'_>| -> Result<()> {
        if let LlmStreamEvent::ReasoningDelta(text) = event {
            reasoning_deltas.push(text.to_string());
        }
        Ok(())
    };

    process_stream_line(
        r#"data: {"type":"response.reasoning_text.delta","delta":"think"}"#,
        &mut state,
        &mut handle_delta,
    )
    .unwrap();
    process_stream_line(
        r#"data: {"type":"response.reasoning_text.delta","delta":" more"}"#,
        &mut state,
        &mut handle_delta,
    )
    .unwrap();

    let response = state.finalize().unwrap();

    assert_eq!(response.metadata.reasoning.as_deref(), Some("think more"));
    assert_eq!(reasoning_deltas, vec!["think", " more"]);
}

#[test]
fn parses_reasoning_text_done_without_deltas() {
    let mut state = AssistantStreamState::default();
    let mut handle_delta = |_event: LlmStreamEvent<'_>| -> Result<()> { Ok(()) };

    process_stream_line(
        r#"data: {"type":"response.reasoning_text.done","text":"final reasoning"}"#,
        &mut state,
        &mut handle_delta,
    )
    .unwrap();

    let response = state.finalize().unwrap();

    assert_eq!(
        response.metadata.reasoning.as_deref(),
        Some("final reasoning")
    );
}

#[test]
fn parses_stream_usage_metadata() {
    let mut state = AssistantStreamState::default();
    let mut handle_delta = |_event: LlmStreamEvent<'_>| -> Result<()> { Ok(()) };

    process_stream_line(
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15,"output_tokens_details":{"reasoning_tokens":1}}}}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

    let response = state.finalize().unwrap();
    let usage = response.metadata.usage.as_ref().unwrap();

    assert_eq!(usage.input_tokens, Some(12));
    assert_eq!(usage.output_tokens, Some(3));
    assert_eq!(usage.total_tokens, Some(15));
    assert_eq!(usage.raw["output_tokens_details"]["reasoning_tokens"], 1);
}

#[test]
fn assembles_streamed_tool_call() {
    let mut state = AssistantStreamState::default();
    let mut tool_call_deltas = Vec::new();
    let mut handle_delta = |event: LlmStreamEvent<'_>| -> Result<()> {
        if let LlmStreamEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_delta,
        } = event
        {
            tool_call_deltas.push((
                index,
                id.map(str::to_string),
                name.map(str::to_string),
                arguments_delta.map(str::to_string),
            ));
        }
        Ok(())
    };

    process_stream_line(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_123","name":"run_shell","arguments":""}}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();
    process_stream_line(
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"command\""}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();
    process_stream_line(
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":":\"ls\"}"}"#,
            &mut state,
            &mut handle_delta,
        )
        .unwrap();

    let response = state.finalize().unwrap();

    assert_eq!(response.finish_reason, Some(FinishReason::ToolCalls));
    assert_eq!(response.metadata.tool_calls.len(), 1);
    assert_eq!(response.metadata.tool_calls[0].id.as_str(), "call_123");
    assert_eq!(response.metadata.tool_calls[0].name(), "run_shell");
    assert_eq!(
        response.metadata.tool_calls[0].arguments(),
        r#"{"command":"ls"}"#
    );
    assert_eq!(
        tool_call_deltas,
        vec![
            (
                0,
                Some("call_123".to_string()),
                Some("run_shell".to_string()),
                None,
            ),
            (
                0,
                Some("call_123".to_string()),
                Some("run_shell".to_string()),
                Some(r#"{"command""#.to_string()),
            ),
            (
                0,
                Some("call_123".to_string()),
                Some("run_shell".to_string()),
                Some(r#":"ls"}"#.to_string()),
            ),
        ]
    );
}

#[test]
fn buffers_split_utf8_bytes() {
    let text = "你";
    let bytes = text.as_bytes();
    let mut byte_buffer = Vec::new();
    let mut text_buffer = String::new();

    byte_buffer.extend_from_slice(&bytes[..1]);
    append_valid_utf8(&mut byte_buffer, &mut text_buffer).unwrap();

    assert!(text_buffer.is_empty());
    assert_eq!(byte_buffer, bytes[..1]);

    byte_buffer.extend_from_slice(&bytes[1..]);
    append_valid_utf8(&mut byte_buffer, &mut text_buffer).unwrap();

    assert_eq!(text_buffer, text);
    assert!(byte_buffer.is_empty());
}

#[test]
fn rejects_invalid_utf8_bytes() {
    let mut byte_buffer = vec![0xff];
    let mut text_buffer = String::new();

    let error = append_valid_utf8(&mut byte_buffer, &mut text_buffer).unwrap_err();

    assert_eq!(
        error.to_string(),
        "responses stream contained invalid utf-8"
    );
}

#[test]
fn rejects_incomplete_final_utf8_bytes() {
    let mut byte_buffer = vec![0xe4];
    let mut text_buffer = String::new();

    let error = finish_utf8(&mut byte_buffer, &mut text_buffer).unwrap_err();

    assert_eq!(
        error.to_string(),
        "responses stream ended with incomplete utf-8"
    );
}

#[test]
fn ignores_done_stream_line() {
    let mut state = AssistantStreamState::default();
    let mut deltas = Vec::new();
    let mut handle_delta = |event: LlmStreamEvent<'_>| -> Result<()> {
        if let LlmStreamEvent::AssistantDelta(text) = event {
            deltas.push(text.to_string());
        }
        Ok(())
    };

    process_stream_line("data: [DONE]", &mut state, &mut handle_delta).unwrap();

    assert!(state.content.is_empty());
    assert!(deltas.is_empty());
}

#[test]
fn accumulates_multiple_stream_lines() {
    let mut buffer = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\
             data: [DONE]\n"
        .to_string();
    let mut state = AssistantStreamState::default();
    let mut deltas = Vec::new();
    let mut handle_delta = |event: LlmStreamEvent<'_>| -> Result<()> {
        if let LlmStreamEvent::AssistantDelta(text) = event {
            deltas.push(text.to_string());
        }
        Ok(())
    };

    process_stream_lines(&mut buffer, &mut state, &mut handle_delta).unwrap();

    assert!(buffer.is_empty());
    assert_eq!(state.content, "Hello");
    assert_eq!(deltas, vec!["Hel", "lo"]);
}
