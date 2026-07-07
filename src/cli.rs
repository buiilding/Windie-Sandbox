//! Startup command parsing for the Windie CLI.
//!
//! This module owns command-line arguments only. It maps raw argv text into
//! typed commands such as `new`, `ls`, `insert`, `update`, `query`, `gateway`,
//! and `bench`. It should not open the database, call Bifrost, or print output.

use std::env;
use std::path::PathBuf;

use crate::conversation::{
    ConversationId, MessageId, Role, ToolCallId, ToolSchema, ToolSchemaName,
};
use crate::llm::ModelName;
use crate::perf::{BenchmarkMode, BenchmarkOptions};
use crate::tool::{ProviderToolName, ToolProviderId};

/// Parsed startup action for one `windie` process.
///
/// This is the CLI boundary's typed contract. Downstream code should match on
/// this enum instead of inspecting raw argv strings.
pub enum Command {
    /// Start the localhost developer API server.
    Api,
    /// Select one message as the active runtime node for a conversation.
    Activate {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    /// List tool calls that are waiting for explicit approval.
    Approvals {
        conversation_id: ConversationId,
    },
    /// Execute one approved tool call.
    ApproveTool {
        conversation_id: ConversationId,
        tool_call_id: ToolCallId,
    },
    /// Attach one provider tool to a conversation.
    AttachTool {
        conversation_id: ConversationId,
        provider_id: ToolProviderId,
        tool_name: ProviderToolName,
    },
    /// Insert one message into a conversation without model inference.
    InsertMessage {
        conversation_id: ConversationId,
        role: Role,
        parts: Vec<InsertPart>,
    },
    /// Insert one conversation-level tool schema.
    InsertToolSchema {
        conversation_id: ConversationId,
        tool_schema: ToolSchema,
    },
    /// Print full read-only runtime state as JSON for developer inspection.
    Inspect {
        conversation_id: ConversationId,
        model: Option<ModelName>,
    },
    /// List provider tools that can be attached to conversations.
    Tools {
        provider_id: Option<ToolProviderId>,
    },
    /// Run one benchmark mode. Conversation mode carries the target
    /// conversation ID; live mode does not.
    Bench {
        mode: BenchmarkMode,
        conversation_id: Option<ConversationId>,
        options: BenchmarkOptions,
    },
    /// Compare two persisted JSON benchmark reports.
    BenchCompare {
        baseline_path: PathBuf,
        current_path: PathBuf,
    },
    /// Copy a conversation from the beginning through one checkpoint message.
    Fork {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    GatewayStart,
    GatewayStop,
    Help,
    Invalid,
    List {
        json: bool,
    },
    /// List models reported by the running Bifrost gateway.
    Models,
    New,
    Noop,
    Query {
        conversation_id: ConversationId,
        model: Option<ModelName>,
    },
    RemoveConversation(ConversationId),
    RemoveMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    RemoveSystemPrompt(ConversationId),
    RemoveToolSchema {
        conversation_id: ConversationId,
        name: ToolSchemaName,
    },
    /// Detach one provider-backed tool schema from a conversation.
    DetachTool {
        conversation_id: ConversationId,
        schema_name: ToolSchemaName,
    },
    DenyTool {
        conversation_id: ConversationId,
        tool_call_id: ToolCallId,
    },
    Show(ConversationId),
    Status,
    SetSystemPrompt {
        conversation_id: ConversationId,
        text: String,
    },
    /// Persist the conversation model used by future queries.
    SetModel {
        conversation_id: ConversationId,
        model: ModelName,
    },
    Truncate {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    Tree(ConversationId),
    UpdateMessage {
        conversation_id: ConversationId,
        message_id: MessageId,
        text: String,
    },
    UpdateToolSchema {
        conversation_id: ConversationId,
        current_name: ToolSchemaName,
        tool_schema: ToolSchema,
    },
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One ordered input part from `windie insert`.
pub enum InsertPart {
    Text(String),
    Image(PathBuf),
}

/// Reads process argv and returns the parsed command for this invocation.
pub fn read() -> Command {
    command_from_args(env::args())
}

/// Converts raw CLI tokens into one typed command.
///
/// This parser is intentionally small and explicit. Unsupported shapes return
/// `Command::Invalid` so `main` can show usage and exit with code 2.
fn command_from_args(args: impl IntoIterator<Item = String>) -> Command {
    let mut args = args.into_iter();
    let _program = args.next();
    let args = args.collect::<Vec<_>>();

    if args.first().is_some_and(|arg| arg == "bench") {
        return parse_bench_command(&args[1..]);
    }

    match args.as_slice() {
        [] => Command::Noop,
        [arg] if arg == "--help" || arg == "-h" => Command::Help,
        [arg] if arg == "--version" || arg == "-V" => Command::Version,
        [arg] if arg == "api" => Command::Api,
        [arg] if arg == "tools" => Command::Tools { provider_id: None },
        [arg] if arg == "models" => Command::Models,
        [command, provider_id] if command == "tools" => Command::Tools {
            provider_id: Some(ToolProviderId::new(provider_id.as_str())),
        },
        [command, conversation_id] if command == "approvals" => Command::Approvals {
            conversation_id: ConversationId::new(conversation_id.as_str()),
        },
        [command, conversation_id, tool_call_id] if command == "approve" => Command::ApproveTool {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            tool_call_id: ToolCallId::new(tool_call_id.as_str()),
        },
        [command, conversation_id, subject, provider_id, tool_name]
            if command == "attach" && subject == "tool" =>
        {
            Command::AttachTool {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                provider_id: ToolProviderId::new(provider_id.as_str()),
                tool_name: ProviderToolName::new(tool_name.as_str()),
            }
        }
        [command, conversation_id, subject, schema_name]
            if command == "detach" && subject == "tool" =>
        {
            Command::DetachTool {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                schema_name: ToolSchemaName::new(schema_name.as_str()),
            }
        }
        [command, conversation_id, tool_call_id] if command == "deny" => Command::DenyTool {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            tool_call_id: ToolCallId::new(tool_call_id.as_str()),
        },
        [command, action] if command == "gateway" && action == "start" => Command::GatewayStart,
        [command, action] if command == "gateway" && action == "stop" => Command::GatewayStop,
        [arg] if arg == "new" => Command::New,
        [arg] if arg == "ls" => Command::List { json: false },
        [command, json_flag] if command == "ls" && json_flag == "--json" => {
            Command::List { json: true }
        }
        [command, conversation_id, message_id] if command == "activate" => Command::Activate {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            message_id: MessageId::new(message_id.as_str()),
        },
        [command, conversation_id] if command == "show" => {
            Command::Show(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id] if command == "tree" => {
            Command::Tree(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id, json_flag] if command == "inspect" && json_flag == "--json" => {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                model: None,
            }
        }
        [command, conversation_id, json_flag, model_flag, model]
            if command == "inspect" && json_flag == "--json" && model_flag == "--model" =>
        {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                model: Some(ModelName::new(model.as_str())),
            }
        }
        [command, rest @ ..] if command == "insert" => parse_insert_command(rest),
        [command, rest @ ..] if command == "update" => parse_update_command(rest),
        [command, conversation_id] if command == "rm" => {
            Command::RemoveConversation(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id, subject, message_id]
            if command == "rm" && subject == "message" =>
        {
            Command::RemoveMessage {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                message_id: MessageId::new(message_id.as_str()),
            }
        }
        [command, conversation_id, subject] if command == "rm" && subject == "systemprompt" => {
            Command::RemoveSystemPrompt(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id, subject, name] if command == "rm" && subject == "toolschema" => {
            Command::RemoveToolSchema {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                name: ToolSchemaName::new(name.as_str()),
            }
        }
        [command, conversation_id, message_id] if command == "truncate" => Command::Truncate {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            message_id: MessageId::new(message_id.as_str()),
        },
        [command, conversation_id, message_id] if command == "fork" => Command::Fork {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            message_id: MessageId::new(message_id.as_str()),
        },
        [command, conversation_id, subject, text_flag, text]
            if command == "set" && subject == "systemprompt" && text_flag == "--text" =>
        {
            Command::SetSystemPrompt {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                text: text.to_string(),
            }
        }
        [command, conversation_id, subject, model] if command == "set" && subject == "model" => {
            Command::SetModel {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                model: ModelName::new(model.as_str()),
            }
        }
        [command, conversation_id] if command == "query" => Command::Query {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            model: None,
        },
        [command, conversation_id, model_flag, model]
            if command == "query" && model_flag == "--model" =>
        {
            Command::Query {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                model: Some(ModelName::new(model.as_str())),
            }
        }
        [arg] if arg == "status" => Command::Status,
        _ => Command::Invalid,
    }
}

/// Parses object inserts under one conversation.
fn parse_insert_command(args: &[String]) -> Command {
    match args {
        [conversation_id, subject, rest @ ..] if subject == "message" => {
            parse_insert_message_command(conversation_id, rest)
        }
        [conversation_id, subject, rest @ ..] if subject == "toolschema" => {
            parse_insert_tool_schema_command(conversation_id, rest)
        }
        _ => Command::Invalid,
    }
}

/// Parses `windie insert <conversation_id> message --role <role> [--text <text>] [--image <path>]...`.
fn parse_insert_message_command(conversation_id: &str, args: &[String]) -> Command {
    let mut role = None;
    let mut parts = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--role") => {
                let Some(value) = args.get(index + 1) else {
                    return Command::Invalid;
                };
                if role.is_some() {
                    return Command::Invalid;
                }
                role = parse_role(value);
                index += 2;
            }
            Some("--text") => {
                let Some(value) = args.get(index + 1) else {
                    return Command::Invalid;
                };
                parts.push(InsertPart::Text(value.to_string()));
                index += 2;
            }
            Some("--image") => {
                let Some(value) = args.get(index + 1) else {
                    return Command::Invalid;
                };
                parts.push(InsertPart::Image(PathBuf::from(value)));
                index += 2;
            }
            _ => return Command::Invalid,
        }
    }

    let Some(role) = role else {
        return Command::Invalid;
    };
    if parts.is_empty() || parts.iter().all(empty_text_part) {
        return Command::Invalid;
    }

    Command::InsertMessage {
        conversation_id: ConversationId::new(conversation_id),
        role,
        parts,
    }
}

/// Parses `windie insert <conversation_id> toolschema --name <name> --description <text> --parameters <json>`.
fn parse_insert_tool_schema_command(conversation_id: &str, args: &[String]) -> Command {
    let Some(tool_schema) = parse_tool_schema_flags(args) else {
        return Command::Invalid;
    };

    Command::InsertToolSchema {
        conversation_id: ConversationId::new(conversation_id),
        tool_schema,
    }
}

/// Parses object updates under one conversation.
fn parse_update_command(args: &[String]) -> Command {
    match args {
        [conversation_id, subject, message_id, text_flag, text]
            if subject == "message" && text_flag == "--text" =>
        {
            Command::UpdateMessage {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                message_id: MessageId::new(message_id.as_str()),
                text: text.to_string(),
            }
        }
        [conversation_id, subject, current_name, rest @ ..] if subject == "toolschema" => {
            let Some(tool_schema) = parse_tool_schema_flags(rest) else {
                return Command::Invalid;
            };

            Command::UpdateToolSchema {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                current_name: ToolSchemaName::new(current_name.as_str()),
                tool_schema,
            }
        }
        _ => Command::Invalid,
    }
}

/// Parses a complete tool schema flag set.
fn parse_tool_schema_flags(args: &[String]) -> Option<ToolSchema> {
    let mut name = None;
    let mut description = None;
    let mut parameters = None;
    let mut index = 0;

    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--name") => {
                if name.is_some() {
                    return None;
                }
                name = args
                    .get(index + 1)
                    .map(|value| ToolSchemaName::new(value.as_str()));
                index += 2;
            }
            Some("--description") => {
                if description.is_some() {
                    return None;
                }
                description = args.get(index + 1).cloned();
                index += 2;
            }
            Some("--parameters") => {
                if parameters.is_some() {
                    return None;
                }
                parameters = args
                    .get(index + 1)
                    .and_then(|value| serde_json::from_str(value).ok());
                index += 2;
            }
            _ => return None,
        }
    }

    let tool_schema = ToolSchema {
        name: name?,
        description: description?,
        parameters: parameters?,
    };
    if !tool_schema.name.is_valid() || !tool_schema.has_valid_description() {
        return None;
    }

    Some(tool_schema)
}

/// Returns whether an insert part carries no user-visible input.
fn empty_text_part(part: &InsertPart) -> bool {
    match part {
        InsertPart::Text(text) => text.is_empty(),
        InsertPart::Image(_) => false,
    }
}

/// Parses benchmark commands and their optional output controls.
///
/// `--runs` repeats local measurements so users can compare median/p95 values
/// across code changes. `--json` writes a persistent artifact to stdout.
fn parse_bench_command(args: &[String]) -> Command {
    if let [command, baseline_path, current_path] = args
        && command == "compare"
    {
        return Command::BenchCompare {
            baseline_path: PathBuf::from(baseline_path),
            current_path: PathBuf::from(current_path),
        };
    }

    let mut index = 0;
    let (mode, conversation_id) = match args.get(index).map(String::as_str) {
        None => return Command::Invalid,
        Some("live") => {
            index += 1;
            (BenchmarkMode::Live, None)
        }
        Some("runtime") => {
            index += 1;
            (BenchmarkMode::Runtime, None)
        }
        Some("ls") => return Command::Invalid,
        Some(argument) if argument.starts_with("--") => return Command::Invalid,
        Some(conversation_id) => {
            index += 1;
            (
                BenchmarkMode::Conversation,
                Some(ConversationId::new(conversation_id)),
            )
        }
    };

    let Some(options) = parse_benchmark_options(&args[index..]) else {
        return Command::Invalid;
    };

    Command::Bench {
        mode,
        conversation_id,
        options,
    }
}

/// Parses optional benchmark flags after the mode/conversation selector.
fn parse_benchmark_options(args: &[String]) -> Option<BenchmarkOptions> {
    let mut options = BenchmarkOptions::default();
    let mut index = 0;

    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--json") => {
                options.json = true;
                index += 1;
            }
            Some("--runs") => {
                let runs = args.get(index + 1)?.parse::<usize>().ok()?;
                if runs == 0 {
                    return None;
                }

                options.runs = runs;
                index += 2;
            }
            _ => return None,
        }
    }

    Some(options)
}

/// Converts CLI role text into the typed role accepted by the conversation
/// model.
fn parse_role(role: &str) -> Option<Role> {
    match role {
        "user" => Some(Role::User),
        "assistant" => Some(Role::Assistant),
        "tool" => Some(Role::Tool),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
            "call-id".to_string(),
        ]);

        assert!(matches!(
            command,
            Command::ApproveTool {
                conversation_id,
                tool_call_id,
            } if conversation_id.as_str() == "conversation-id" && tool_call_id.as_str() == "call-id"
        ));
    }

    #[test]
    fn reads_deny_tool_command() {
        let command = command_from_args([
            "windie".to_string(),
            "deny".to_string(),
            "conversation-id".to_string(),
            "call-id".to_string(),
        ]);

        assert!(matches!(
            command,
            Command::DenyTool {
                conversation_id,
                tool_call_id,
            } if conversation_id.as_str() == "conversation-id" && tool_call_id.as_str() == "call-id"
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
        let command =
            command_from_args(["windie".to_string(), "ls".to_string(), "--json".to_string()]);

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

        assert!(
            matches!(command, Command::RemoveConversation(id) if id.as_str() == "conversation-id")
        );
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

        assert!(
            matches!(command, Command::RemoveSystemPrompt(id) if id.as_str() == "conversation-id")
        );
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
        let command =
            command_from_args(["windie".to_string(), "bench".to_string(), "ls".to_string()]);

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
}
