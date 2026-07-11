//! Responses SSE decoding tests.

use super::*;

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
