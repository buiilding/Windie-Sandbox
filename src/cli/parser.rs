//! Top-level argv dispatch for the CLI parser.

use super::*;

/// Reads process argv and returns the parsed command for this invocation.
pub fn read() -> Command {
    command_from_args(std::env::args())
}

/// Converts raw CLI tokens into one typed command.
///
/// This parser is intentionally small and explicit. Unsupported shapes return
/// `Command::Invalid` so `main` can show usage and exit with code 2.
pub(super) fn command_from_args(args: impl IntoIterator<Item = String>) -> Command {
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
        [arg] if arg == "inspector" => Command::Inspector,
        [command, target] if command == "install" => Command::Install {
            target: target.to_string(),
        },
        [command, rest @ ..] if command == "env" => parse_env_command(rest),
        [command, subject, rest @ ..] if command == "compare" && subject == "baseline" => {
            parse_baseline_command(rest, BaselineCommand::Compare)
        }
        [command, subject, rest @ ..] if command == "update" && subject == "baseline" => {
            parse_baseline_command(rest, BaselineCommand::Update)
        }
        [command, rest @ ..] if command == "run" => parse_run_command(rest),
        [arg] if arg == "tools" => Command::Tools { provider_id: None },
        [arg] if arg == "models" => Command::Models,
        [command, provider_id] if command == "tools" => Command::Tools {
            provider_id: Some(ToolProviderId::new(provider_id.as_str())),
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
        [command, action] if command == "gateway" && action == "start" => Command::GatewayStart,
        [command, action] if command == "gateway" && action == "stop" => Command::GatewayStop,
        [arg] if arg == "new" => Command::New,
        [arg] if arg == "ls" => Command::List { json: false },
        [command, json_flag] if command == "ls" && json_flag == "--json" => {
            Command::List { json: true }
        }
        [command, conversation_id] if command == "show" => {
            Command::Show(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id] if command == "tree" => {
            Command::Tree(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id, json_flag] if command == "inspect" && json_flag == "--json" => {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                head_message_id: None,
                model: None,
            }
        }
        [command, conversation_id, json_flag, model_flag, model]
            if command == "inspect" && json_flag == "--json" && model_flag == "--model" =>
        {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                head_message_id: None,
                model: Some(ModelName::new(model.as_str())),
            }
        }
        [
            command,
            conversation_id,
            json_flag,
            head_flag,
            head_message_id,
        ] if command == "inspect" && json_flag == "--json" && head_flag == "--head" => {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                head_message_id: Some(MessageId::new(head_message_id.as_str())),
                model: None,
            }
        }
        [
            command,
            conversation_id,
            json_flag,
            head_flag,
            head_message_id,
            model_flag,
            model,
        ] if command == "inspect"
            && json_flag == "--json"
            && head_flag == "--head"
            && model_flag == "--model" =>
        {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                head_message_id: Some(MessageId::new(head_message_id.as_str())),
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
        [arg] if arg == "status" => Command::Status,
        _ => Command::Invalid,
    }
}
