//! Message command parsing.

use super::*;

/// Parses object inserts under one conversation.
pub(super) fn parse_insert_command(args: &[String]) -> Command {
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
    let mut head_message_id = None;
    let mut role = None;
    let mut parts = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--head") => {
                let Some(value) = args.get(index + 1) else {
                    return Command::Invalid;
                };
                if head_message_id.is_some() {
                    return Command::Invalid;
                }
                head_message_id = Some(MessageId::new(value.as_str()));
                index += 2;
            }
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
        head_message_id,
        role,
        parts,
    }
}

/// Parses object updates under one conversation.
pub(super) fn parse_update_command(args: &[String]) -> Command {
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

/// Returns whether an insert part carries no user-visible input.
fn empty_text_part(part: &InsertPart) -> bool {
    match part {
        InsertPart::Text(text) => text.is_empty(),
        InsertPart::Image(_) => false,
    }
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
