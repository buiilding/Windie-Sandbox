//! Startup command parsing for the Windie CLI.
//!
//! This module owns command-line arguments only. It maps raw argv text into
//! typed commands such as `new`, `ls`, `append`, `update`, `query`, `gateway`,
//! and `bench`. It should not open the database, call Bifrost, or print output.

use std::env;

use crate::conversation::{ConversationId, MessageId, Role};
use crate::llm::ModelName;
use crate::perf::BenchmarkMode;

/// Parsed startup action for one `windie` process.
///
/// This is the CLI boundary's typed contract. Downstream code should match on
/// this enum instead of inspecting raw argv strings.
pub enum Command {
    /// Add one message to a conversation without model inference.
    Append {
        conversation_id: ConversationId,
        role: Role,
        text: String,
    },
    /// Run one benchmark mode. Conversation mode carries the target
    /// conversation ID; local and live modes do not.
    Bench {
        mode: BenchmarkMode,
        conversation_id: Option<ConversationId>,
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
    List,
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
    Show(ConversationId),
    Status,
    Truncate {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    Update {
        conversation_id: ConversationId,
        message_id: MessageId,
        text: String,
    },
    Version,
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

    match args.as_slice() {
        [] => Command::Noop,
        [arg] if arg == "--help" || arg == "-h" => Command::Help,
        [arg] if arg == "--version" || arg == "-V" => Command::Version,
        [arg] if arg == "bench" => Command::Bench {
            mode: BenchmarkMode::Local,
            conversation_id: None,
        },
        [command, mode] if command == "bench" && mode == "live" => Command::Bench {
            mode: BenchmarkMode::Live,
            conversation_id: None,
        },
        [command, conversation_id] if command == "bench" => Command::Bench {
            mode: BenchmarkMode::Conversation,
            conversation_id: Some(ConversationId::new(conversation_id.as_str())),
        },
        [command, action] if command == "gateway" && action == "start" => Command::GatewayStart,
        [command, action] if command == "gateway" && action == "stop" => Command::GatewayStop,
        [arg] if arg == "new" => Command::New,
        [arg] if arg == "ls" => Command::List,
        [command, conversation_id] if command == "show" => {
            Command::Show(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id, role_flag, role, text_flag, text]
            if command == "append" && role_flag == "--role" && text_flag == "--text" =>
        {
            let Some(role) = parse_role(role) else {
                return Command::Invalid;
            };

            Command::Append {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                role,
                text: text.to_string(),
            }
        }
        [command, conversation_id, message_id, text_flag, text]
            if command == "update" && text_flag == "--text" =>
        {
            Command::Update {
                conversation_id: ConversationId::new(conversation_id.as_str()),
                message_id: MessageId::new(message_id.as_str()),
                text: text.to_string(),
            }
        }
        [command, conversation_id] if command == "rm" => {
            Command::RemoveConversation(ConversationId::new(conversation_id.as_str()))
        }
        [command, conversation_id, message_id] if command == "rm" => Command::RemoveMessage {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            message_id: MessageId::new(message_id.as_str()),
        },
        [command, conversation_id, message_id] if command == "truncate" => Command::Truncate {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            message_id: MessageId::new(message_id.as_str()),
        },
        [command, conversation_id, message_id] if command == "fork" => Command::Fork {
            conversation_id: ConversationId::new(conversation_id.as_str()),
            message_id: MessageId::new(message_id.as_str()),
        },
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

/// Converts CLI role text into the typed role accepted by the conversation
/// model.
fn parse_role(role: &str) -> Option<Role> {
    match role {
        "system" => Some(Role::System),
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

        assert!(matches!(command, Command::List));
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
    fn rejects_show_without_id() {
        let command = command_from_args(["windie".to_string(), "show".to_string()]);

        assert!(matches!(command, Command::Invalid));
    }

    #[test]
    fn reads_append_command() {
        let command = command_from_args([
            "windie".to_string(),
            "append".to_string(),
            "conversation-id".to_string(),
            "--role".to_string(),
            "user".to_string(),
            "--text".to_string(),
            "hello".to_string(),
        ]);

        assert!(matches!(
            command,
            Command::Append {
                conversation_id,
                role: Role::User,
                text,
            } if conversation_id.as_str() == "conversation-id" && text == "hello"
        ));
    }

    #[test]
    fn rejects_append_with_unknown_role() {
        let command = command_from_args([
            "windie".to_string(),
            "append".to_string(),
            "conversation-id".to_string(),
            "--role".to_string(),
            "owner".to_string(),
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
            "message-id".to_string(),
            "--text".to_string(),
            "new text".to_string(),
        ]);

        assert!(matches!(
            command,
            Command::Update {
                conversation_id,
                message_id,
                text,
            } if conversation_id.as_str() == "conversation-id"
                && message_id.as_str() == "message-id"
                && text == "new text"
        ));
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
    fn reads_status_command() {
        let command = command_from_args(["windie".to_string(), "status".to_string()]);

        assert!(matches!(command, Command::Status));
    }

    #[test]
    fn reads_bench_command() {
        let command = command_from_args(["windie".to_string(), "bench".to_string()]);

        assert!(matches!(
            command,
            Command::Bench {
                mode: BenchmarkMode::Local,
                conversation_id: None,
            }
        ));
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
            }
        ));
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
            } if id.as_str() == "conversation-id"
        ));
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
