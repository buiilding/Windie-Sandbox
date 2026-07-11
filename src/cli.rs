//! Startup command parsing for the Windie CLI.
//!
//! The facade maps raw argv text into typed commands such as `new`, `ls`,
//! `insert`, `update`, `query`, `gateway`, and `bench`. The `execute` child is
//! the CLI adapter that opens the database and delegates to shared operations;
//! business rules stay outside this module.

mod execute;

pub use execute::execute;

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
    /// Inspect installation paths and external integration prerequisites.
    Doctor,
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

    let Some((command, rest)) = args.split_first() else {
        return Command::Noop;
    };
    match command.as_str() {
        "bench" => parse_bench_command(rest),
        "insert" => parse_insert_command(rest),
        "update" => parse_update_command(rest),
        "rm" => parse_remove_command(rest),
        "set" => parse_set_command(rest),
        "query" => parse_query_command(rest),
        "inspect" => parse_inspect_command(rest),
        "attach" => parse_attach_command(rest),
        "detach" => parse_detach_command(rest),
        "approve" => parse_tool_decision(rest, true),
        "deny" => parse_tool_decision(rest, false),
        _ => parse_simple_command(command, rest),
    }
}

fn parse_simple_command(command: &str, args: &[String]) -> Command {
    match (command, args) {
        ("--help" | "-h", []) => Command::Help,
        ("--version" | "-V", []) => Command::Version,
        ("api", []) => Command::Api,
        ("doctor", []) => Command::Doctor,
        ("models", []) => Command::Models,
        ("new", []) => Command::New,
        ("status", []) => Command::Status,
        ("tools", []) => Command::Tools { provider_id: None },
        ("tools", [provider_id]) => Command::Tools {
            provider_id: Some(ToolProviderId::new(provider_id)),
        },
        ("approvals", [conversation_id]) => Command::Approvals {
            conversation_id: ConversationId::new(conversation_id),
        },
        ("gateway", [action]) if action == "start" => Command::GatewayStart,
        ("gateway", [action]) if action == "stop" => Command::GatewayStop,
        ("ls", []) => Command::List { json: false },
        ("ls", [flag]) if flag == "--json" => Command::List { json: true },
        ("show", [conversation_id]) => Command::Show(ConversationId::new(conversation_id)),
        ("tree", [conversation_id]) => Command::Tree(ConversationId::new(conversation_id)),
        ("activate", [conversation_id, message_id]) => Command::Activate {
            conversation_id: ConversationId::new(conversation_id),
            message_id: MessageId::new(message_id),
        },
        ("truncate", [conversation_id, message_id]) => Command::Truncate {
            conversation_id: ConversationId::new(conversation_id),
            message_id: MessageId::new(message_id),
        },
        ("fork", [conversation_id, message_id]) => Command::Fork {
            conversation_id: ConversationId::new(conversation_id),
            message_id: MessageId::new(message_id),
        },
        _ => Command::Invalid,
    }
}

fn parse_tool_decision(args: &[String], approve: bool) -> Command {
    let [conversation_id, tool_call_id] = args else {
        return Command::Invalid;
    };
    if approve {
        Command::ApproveTool {
            conversation_id: ConversationId::new(conversation_id),
            tool_call_id: ToolCallId::new(tool_call_id),
        }
    } else {
        Command::DenyTool {
            conversation_id: ConversationId::new(conversation_id),
            tool_call_id: ToolCallId::new(tool_call_id),
        }
    }
}

fn parse_attach_command(args: &[String]) -> Command {
    match args {
        [conversation_id, subject, provider_id, tool_name] if subject == "tool" => {
            Command::AttachTool {
                conversation_id: ConversationId::new(conversation_id),
                provider_id: ToolProviderId::new(provider_id),
                tool_name: ProviderToolName::new(tool_name),
            }
        }
        _ => Command::Invalid,
    }
}

fn parse_detach_command(args: &[String]) -> Command {
    match args {
        [conversation_id, subject, schema_name] if subject == "tool" => Command::DetachTool {
            conversation_id: ConversationId::new(conversation_id),
            schema_name: ToolSchemaName::new(schema_name),
        },
        _ => Command::Invalid,
    }
}

fn parse_inspect_command(args: &[String]) -> Command {
    match args {
        [conversation_id, json_flag] if json_flag == "--json" => Command::Inspect {
            conversation_id: ConversationId::new(conversation_id),
            model: None,
        },
        [conversation_id, json_flag, model_flag, model]
            if json_flag == "--json" && model_flag == "--model" =>
        {
            Command::Inspect {
                conversation_id: ConversationId::new(conversation_id),
                model: Some(ModelName::new(model)),
            }
        }
        _ => Command::Invalid,
    }
}

fn parse_query_command(args: &[String]) -> Command {
    match args {
        [conversation_id] => Command::Query {
            conversation_id: ConversationId::new(conversation_id),
            model: None,
        },
        [conversation_id, model_flag, model] if model_flag == "--model" => Command::Query {
            conversation_id: ConversationId::new(conversation_id),
            model: Some(ModelName::new(model)),
        },
        _ => Command::Invalid,
    }
}

fn parse_set_command(args: &[String]) -> Command {
    match args {
        [conversation_id, subject, text_flag, text]
            if subject == "systemprompt" && text_flag == "--text" =>
        {
            Command::SetSystemPrompt {
                conversation_id: ConversationId::new(conversation_id),
                text: text.to_string(),
            }
        }
        [conversation_id, subject, model] if subject == "model" => Command::SetModel {
            conversation_id: ConversationId::new(conversation_id),
            model: ModelName::new(model),
        },
        _ => Command::Invalid,
    }
}

fn parse_remove_command(args: &[String]) -> Command {
    match args {
        [conversation_id] => Command::RemoveConversation(ConversationId::new(conversation_id)),
        [conversation_id, subject, message_id] if subject == "message" => Command::RemoveMessage {
            conversation_id: ConversationId::new(conversation_id),
            message_id: MessageId::new(message_id),
        },
        [conversation_id, subject] if subject == "systemprompt" => {
            Command::RemoveSystemPrompt(ConversationId::new(conversation_id))
        }
        [conversation_id, subject, name] if subject == "toolschema" => Command::RemoveToolSchema {
            conversation_id: ConversationId::new(conversation_id),
            name: ToolSchemaName::new(name),
        },
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
mod tests;
