//! CLI parsing tests.

use super::*;

#[test]
fn reads_noop_command_by_default() {
    let command = command_from_args(["windie".to_string()]);

    assert!(matches!(command, Command::Noop));
}

#[test]
fn reads_long_help_command() {
    let command = command_from_args(["windie".to_string(), "--help".to_string()]);

    assert!(matches!(command, Command::Help));
}

#[test]
fn reads_short_help_command() {
    let command = command_from_args(["windie".to_string(), "-h".to_string()]);

    assert!(matches!(command, Command::Help));
}

#[test]
fn reads_long_version_command() {
    let command = command_from_args(["windie".to_string(), "--version".to_string()]);

    assert!(matches!(command, Command::Version));
}

#[test]
fn reads_short_version_command() {
    let command = command_from_args(["windie".to_string(), "-V".to_string()]);

    assert!(matches!(command, Command::Version));
}

#[test]
fn reads_api_command() {
    let command = command_from_args(["windie".to_string(), "api".to_string()]);

    assert!(matches!(command, Command::Api));
}

#[test]
fn reads_doctor_command() {
    let command = command_from_args(["windie".to_string(), "doctor".to_string()]);

    assert!(matches!(command, Command::Doctor));
}

#[test]
fn reads_tools_command() {
    let command = command_from_args(["windie".to_string(), "tools".to_string()]);

    assert!(matches!(command, Command::Tools { provider_id: None }));
}

#[test]
fn reads_models_command() {
    let command = command_from_args(["windie".to_string(), "models".to_string()]);

    assert!(matches!(command, Command::Models));
}

#[test]
fn reads_provider_tools_command() {
    let command = command_from_args([
        "windie".to_string(),
        "tools".to_string(),
        "windie".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Tools {
            provider_id: Some(provider_id)
        } if provider_id.as_str() == "windie"
    ));
}

#[test]
fn reads_attach_tool_command() {
    let command = command_from_args([
        "windie".to_string(),
        "attach".to_string(),
        "conversation-id".to_string(),
        "tool".to_string(),
        "windie".to_string(),
        "run_shell".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::AttachTool {
            conversation_id,
            provider_id,
            tool_name,
        } if conversation_id.as_str() == "conversation-id"
            && provider_id.as_str() == "windie"
            && tool_name.as_str() == "run_shell"
    ));
}

#[test]
fn reads_detach_tool_command() {
    let command = command_from_args([
        "windie".to_string(),
        "detach".to_string(),
        "conversation-id".to_string(),
        "tool".to_string(),
        "run_shell".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::DetachTool {
            conversation_id,
            schema_name,
        } if conversation_id.as_str() == "conversation-id"
            && schema_name.as_str() == "run_shell"
    ));
}

#[test]
fn reads_approvals_command() {
    let command = command_from_args([
        "windie".to_string(),
        "approvals".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Approvals { conversation_id } if conversation_id.as_str() == "conversation-id"
    ));
}

#[test]
fn reads_approve_tool_command() {
    let command = command_from_args([
        "windie".to_string(),
        "approve".to_string(),
        "conversation-id".to_string(),
        "assistant-id".to_string(),
        "call-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::ApproveTool {
            conversation_id,
            target,
        } if conversation_id.as_str() == "conversation-id"
            && target.assistant_message_id.as_str() == "assistant-id"
            && target.tool_call_id.as_str() == "call-id"
    ));
}

#[test]
fn reads_deny_tool_command() {
    let command = command_from_args([
        "windie".to_string(),
        "deny".to_string(),
        "conversation-id".to_string(),
        "assistant-id".to_string(),
        "call-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::DenyTool {
            conversation_id,
            target,
        } if conversation_id.as_str() == "conversation-id"
            && target.assistant_message_id.as_str() == "assistant-id"
            && target.tool_call_id.as_str() == "call-id"
    ));
}

#[test]
fn rejects_combined_top_level_options() {
    let command = command_from_args([
        "windie".to_string(),
        "--version".to_string(),
        "--help".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_new_command() {
    let command = command_from_args(["windie".to_string(), "new".to_string()]);

    assert!(matches!(command, Command::New));
}

#[test]
fn reads_gateway_start_command() {
    let command = command_from_args([
        "windie".to_string(),
        "gateway".to_string(),
        "start".to_string(),
    ]);

    assert!(matches!(command, Command::GatewayStart));
}

#[test]
fn reads_gateway_stop_command() {
    let command = command_from_args([
        "windie".to_string(),
        "gateway".to_string(),
        "stop".to_string(),
    ]);

    assert!(matches!(command, Command::GatewayStop));
}

#[test]
fn rejects_unknown_gateway_command() {
    let command = command_from_args([
        "windie".to_string(),
        "gateway".to_string(),
        "restart".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_ls_command() {
    let command = command_from_args(["windie".to_string(), "ls".to_string()]);

    assert!(matches!(command, Command::List { json: false }));
}

#[test]
fn reads_ls_json_command() {
    let command = command_from_args(["windie".to_string(), "ls".to_string(), "--json".to_string()]);

    assert!(matches!(command, Command::List { json: true }));
}

#[test]
fn rejects_list_command() {
    let command = command_from_args(["windie".to_string(), "list".to_string()]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_show_command() {
    let command = command_from_args([
        "windie".to_string(),
        "show".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(command, Command::Show(id) if id.as_str() == "conversation-id"));
}

#[test]
fn reads_tree_command() {
    let command = command_from_args([
        "windie".to_string(),
        "tree".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(command, Command::Tree(id) if id.as_str() == "conversation-id"));
}

#[test]
fn reads_activate_command() {
    let command = command_from_args([
        "windie".to_string(),
        "activate".to_string(),
        "conversation-id".to_string(),
        "message-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Activate {
            conversation_id,
            message_id,
        } if conversation_id.as_str() == "conversation-id" && message_id.as_str() == "message-id"
    ));
}

#[test]
fn rejects_show_without_id() {
    let command = command_from_args(["windie".to_string(), "show".to_string()]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_insert_command() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "--role".to_string(),
        "user".to_string(),
        "--text".to_string(),
        "hello".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::InsertMessage {
            conversation_id,
            role: Role::User,
            parts,
        } if conversation_id.as_str() == "conversation-id"
            && parts == vec![InsertPart::Text("hello".to_string())]
    ));
}

#[test]
fn reads_insert_command_with_image() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "--role".to_string(),
        "user".to_string(),
        "--text".to_string(),
        "what is this?".to_string(),
        "--image".to_string(),
        "image.png".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::InsertMessage {
            conversation_id,
            role: Role::User,
            parts,
        } if conversation_id.as_str() == "conversation-id"
            && parts == vec![
                InsertPart::Text("what is this?".to_string()),
                InsertPart::Image(PathBuf::from("image.png")),
            ]
    ));
}

#[test]
fn reads_insert_command_with_multiple_images() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "--role".to_string(),
        "user".to_string(),
        "--text".to_string(),
        "compare these".to_string(),
        "--image".to_string(),
        "first.png".to_string(),
        "--image".to_string(),
        "second.png".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::InsertMessage {
            conversation_id,
            role: Role::User,
            parts,
        } if conversation_id.as_str() == "conversation-id"
            && parts == vec![
                InsertPart::Text("compare these".to_string()),
                InsertPart::Image(PathBuf::from("first.png")),
                InsertPart::Image(PathBuf::from("second.png")),
            ]
    ));
}

#[test]
fn reads_insert_command_with_interleaved_text_and_images() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "--role".to_string(),
        "user".to_string(),
        "--text".to_string(),
        "first".to_string(),
        "--image".to_string(),
        "first.png".to_string(),
        "--text".to_string(),
        "second".to_string(),
        "--image".to_string(),
        "second.png".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::InsertMessage {
            conversation_id,
            role: Role::User,
            parts,
        } if conversation_id.as_str() == "conversation-id"
            && parts == vec![
                InsertPart::Text("first".to_string()),
                InsertPart::Image(PathBuf::from("first.png")),
                InsertPart::Text("second".to_string()),
                InsertPart::Image(PathBuf::from("second.png")),
            ]
    ));
}

#[test]
fn reads_insert_command_with_only_image() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "--role".to_string(),
        "user".to_string(),
        "--image".to_string(),
        "image.png".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::InsertMessage {
            conversation_id,
            role: Role::User,
            parts,
        } if conversation_id.as_str() == "conversation-id"
            && parts == vec![InsertPart::Image(PathBuf::from("image.png"))]
    ));
}

#[test]
fn rejects_insert_with_unknown_role() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "--role".to_string(),
        "owner".to_string(),
        "--text".to_string(),
        "hello".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn rejects_append_command() {
    let command = command_from_args([
        "windie".to_string(),
        "append".to_string(),
        "conversation-id".to_string(),
        "--role".to_string(),
        "user".to_string(),
        "--text".to_string(),
        "hello".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_update_command() {
    let command = command_from_args([
        "windie".to_string(),
        "update".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "message-id".to_string(),
        "--text".to_string(),
        "new text".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::UpdateMessage {
            conversation_id,
            message_id,
            text,
        } if conversation_id.as_str() == "conversation-id"
            && message_id.as_str() == "message-id"
            && text == "new text"
    ));
}

#[test]
fn reads_insert_tool_schema_command() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "toolschema".to_string(),
        "--name".to_string(),
        "run_shell".to_string(),
        "--description".to_string(),
        "Run a shell command".to_string(),
        "--parameters".to_string(),
        r#"{"type":"object"}"#.to_string(),
    ]);

    assert!(matches!(
        command,
        Command::InsertToolSchema {
            conversation_id,
            tool_schema,
        } if conversation_id.as_str() == "conversation-id"
            && tool_schema.name.as_str() == "run_shell"
            && tool_schema.description == "Run a shell command"
            && tool_schema.parameters == serde_json::json!({"type":"object"})
    ));
}

#[test]
fn reads_update_tool_schema_command() {
    let command = command_from_args([
        "windie".to_string(),
        "update".to_string(),
        "conversation-id".to_string(),
        "toolschema".to_string(),
        "run_shell".to_string(),
        "--name".to_string(),
        "shell".to_string(),
        "--description".to_string(),
        "Run command".to_string(),
        "--parameters".to_string(),
        r#"{"type":"object"}"#.to_string(),
    ]);

    assert!(matches!(
        command,
        Command::UpdateToolSchema {
            conversation_id,
            current_name,
            tool_schema,
        } if conversation_id.as_str() == "conversation-id"
            && current_name.as_str() == "run_shell"
            && tool_schema.name.as_str() == "shell"
            && tool_schema.description == "Run command"
            && tool_schema.parameters == serde_json::json!({"type":"object"})
    ));
}

#[test]
fn rejects_tool_schema_with_empty_name() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "toolschema".to_string(),
        "--name".to_string(),
        String::new(),
        "--description".to_string(),
        "Run a shell command".to_string(),
        "--parameters".to_string(),
        r#"{"type":"object"}"#.to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn rejects_tool_schema_with_invalid_name_characters() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "toolschema".to_string(),
        "--name".to_string(),
        "run shell".to_string(),
        "--description".to_string(),
        "Run a shell command".to_string(),
        "--parameters".to_string(),
        r#"{"type":"object"}"#.to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn rejects_tool_schema_with_empty_description() {
    let command = command_from_args([
        "windie".to_string(),
        "insert".to_string(),
        "conversation-id".to_string(),
        "toolschema".to_string(),
        "--name".to_string(),
        "run_shell".to_string(),
        "--description".to_string(),
        "   ".to_string(),
        "--parameters".to_string(),
        r#"{"type":"object"}"#.to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_remove_conversation_command() {
    let command = command_from_args([
        "windie".to_string(),
        "rm".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(command, Command::RemoveConversation(id) if id.as_str() == "conversation-id"));
}

#[test]
fn reads_remove_message_command() {
    let command = command_from_args([
        "windie".to_string(),
        "rm".to_string(),
        "conversation-id".to_string(),
        "message".to_string(),
        "message-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::RemoveMessage {
            conversation_id,
            message_id,
        } if conversation_id.as_str() == "conversation-id" && message_id.as_str() == "message-id"
    ));
}

#[test]
fn reads_remove_systemprompt_command() {
    let command = command_from_args([
        "windie".to_string(),
        "rm".to_string(),
        "conversation-id".to_string(),
        "systemprompt".to_string(),
    ]);

    assert!(matches!(command, Command::RemoveSystemPrompt(id) if id.as_str() == "conversation-id"));
}

#[test]
fn reads_remove_tool_schema_command() {
    let command = command_from_args([
        "windie".to_string(),
        "rm".to_string(),
        "conversation-id".to_string(),
        "toolschema".to_string(),
        "run_shell".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::RemoveToolSchema {
            conversation_id,
            name,
        } if conversation_id.as_str() == "conversation-id" && name.as_str() == "run_shell"
    ));
}

#[test]
fn reads_truncate_command() {
    let command = command_from_args([
        "windie".to_string(),
        "truncate".to_string(),
        "conversation-id".to_string(),
        "message-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Truncate {
            conversation_id,
            message_id,
        } if conversation_id.as_str() == "conversation-id" && message_id.as_str() == "message-id"
    ));
}

#[test]
fn reads_fork_command() {
    let command = command_from_args([
        "windie".to_string(),
        "fork".to_string(),
        "conversation-id".to_string(),
        "message-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Fork {
            conversation_id,
            message_id,
        } if conversation_id.as_str() == "conversation-id" && message_id.as_str() == "message-id"
    ));
}

#[test]
fn reads_query_command() {
    let command = command_from_args([
        "windie".to_string(),
        "query".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Query {
            conversation_id,
            model: None,
        } if conversation_id.as_str() == "conversation-id"
    ));
}

#[test]
fn reads_inspect_json_command() {
    let command = command_from_args([
        "windie".to_string(),
        "inspect".to_string(),
        "conversation-id".to_string(),
        "--json".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Inspect {
            conversation_id,
            model: None,
        } if conversation_id.as_str() == "conversation-id"
    ));
}

#[test]
fn reads_inspect_json_with_model_command() {
    let command = command_from_args([
        "windie".to_string(),
        "inspect".to_string(),
        "conversation-id".to_string(),
        "--json".to_string(),
        "--model".to_string(),
        "anthropic/claude-3-5-haiku".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Inspect {
            conversation_id,
            model: Some(model),
        } if conversation_id.as_str() == "conversation-id"
            && model.as_str() == "anthropic/claude-3-5-haiku"
    ));
}

#[test]
fn rejects_inspect_without_json() {
    let command = command_from_args([
        "windie".to_string(),
        "inspect".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_set_systemprompt_command() {
    let command = command_from_args([
        "windie".to_string(),
        "set".to_string(),
        "conversation-id".to_string(),
        "systemprompt".to_string(),
        "--text".to_string(),
        "You are concise.".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::SetSystemPrompt {
            conversation_id,
            text,
        } if conversation_id.as_str() == "conversation-id" && text == "You are concise."
    ));
}

#[test]
fn rejects_set_systemprompt_without_text() {
    let command = command_from_args([
        "windie".to_string(),
        "set".to_string(),
        "conversation-id".to_string(),
        "systemprompt".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_set_model_command() {
    let command = command_from_args([
        "windie".to_string(),
        "set".to_string(),
        "conversation-id".to_string(),
        "model".to_string(),
        "anthropic/claude-3-5-haiku".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::SetModel {
            conversation_id,
            model,
        } if conversation_id.as_str() == "conversation-id"
            && model.as_str() == "anthropic/claude-3-5-haiku"
    ));
}

#[test]
fn reads_query_with_model_command() {
    let command = command_from_args([
        "windie".to_string(),
        "query".to_string(),
        "conversation-id".to_string(),
        "--model".to_string(),
        "openai/gpt-4o-mini".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Query {
            conversation_id,
            model: Some(model),
        } if conversation_id.as_str() == "conversation-id" && model.as_str() == "openai/gpt-4o-mini"
    ));
}

#[test]
fn reads_query_with_provider_qualified_model() {
    let command = command_from_args([
        "windie".to_string(),
        "query".to_string(),
        "conversation-id".to_string(),
        "--model".to_string(),
        "anthropic/claude-3-5-haiku".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Query {
            conversation_id,
            model: Some(model),
        } if conversation_id.as_str() == "conversation-id"
            && model.as_str() == "anthropic/claude-3-5-haiku"
    ));
}

#[test]
fn reads_status_command() {
    let command = command_from_args(["windie".to_string(), "status".to_string()]);

    assert!(matches!(command, Command::Status));
}

#[test]
fn rejects_bare_bench_command() {
    let command = command_from_args(["windie".to_string(), "bench".to_string()]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_live_bench_command() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "live".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Bench {
            mode: BenchmarkMode::Live,
            conversation_id: None,
            options,
        } if options.runs == 1 && !options.json
    ));
}

#[test]
fn reads_runtime_bench_command() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "runtime".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Bench {
            mode: BenchmarkMode::Runtime,
            conversation_id: None,
            options,
        } if options.runs == 1 && !options.json
    ));
}

#[test]
fn rejects_list_bench_command() {
    let command = command_from_args(["windie".to_string(), "bench".to_string(), "ls".to_string()]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn rejects_list_bench_with_runs_and_json() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "ls".to_string(),
        "--runs".to_string(),
        "10".to_string(),
        "--json".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_conversation_bench_command() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "conversation-id".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Bench {
            mode: BenchmarkMode::Conversation,
            conversation_id: Some(id),
            options,
        } if id.as_str() == "conversation-id" && options.runs == 1 && !options.json
    ));
}

#[test]
fn reads_conversation_bench_with_runs_and_json() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "conversation-id".to_string(),
        "--runs".to_string(),
        "100".to_string(),
        "--json".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::Bench {
            mode: BenchmarkMode::Conversation,
            conversation_id: Some(id),
            options,
        } if id.as_str() == "conversation-id" && options.runs == 100 && options.json
    ));
}

#[test]
fn rejects_bench_options_without_conversation_id() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "--json".to_string(),
        "--runs".to_string(),
        "10".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_bench_compare_command() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "compare".to_string(),
        "baseline.json".to_string(),
        "current.json".to_string(),
    ]);

    assert!(matches!(
        command,
        Command::BenchCompare {
            baseline_path,
            current_path,
        } if baseline_path == std::path::Path::new("baseline.json")
            && current_path == std::path::Path::new("current.json")
    ));
}

#[test]
fn rejects_zero_benchmark_runs() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "--runs".to_string(),
        "0".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn rejects_bench_with_extra_arg() {
    let command = command_from_args([
        "windie".to_string(),
        "bench".to_string(),
        "conversation-id".to_string(),
        "extra".to_string(),
    ]);

    assert!(matches!(command, Command::Invalid));
}

#[test]
fn reads_unknown_command_as_invalid() {
    let command = command_from_args(["windie".to_string(), "whatever".to_string()]);

    assert!(matches!(command, Command::Invalid));
}
