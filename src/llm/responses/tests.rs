//! Responses request serialization tests.

use super::*;
use crate::conversation::{
    ImageAssetId, MessageId, MessageMetadata, Role, ToolCall, ToolCallFunction, ToolCallId,
    ToolCallKind, ToolSchema, ToolSchemaName,
};
use crate::llm::reasoning_request_for_model;

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
fn openai_reasoning_effort_requests_visible_summary() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("openai/gpt-5.5"),
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: None,
        }),
    )
    .unwrap();

    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(reasoning.summary.as_deref(), Some("auto"));
}

#[test]
fn openai_reasoning_preserves_explicit_summary() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("openai/gpt-5.5"),
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: Some("detailed".to_string()),
        }),
    )
    .unwrap();

    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(reasoning.summary.as_deref(), Some("detailed"));
}

#[test]
fn anthropic_reasoning_does_not_request_openai_summary() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("anthropic/claude-fable-5"),
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: None,
        }),
    )
    .unwrap();

    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(reasoning.summary, None);
}

#[test]
fn empty_reasoning_request_stays_absent() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("openai/gpt-5.5"),
        Some(ReasoningRequest {
            effort: None,
            summary: None,
        }),
    );

    assert_eq!(reasoning, None);
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
