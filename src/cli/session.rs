//! Session command parsing.

use super::*;

/// Parses run-owned execution commands.
pub(super) fn parse_run_command(args: &[String]) -> Command {
    match args {
        [action, conversation_id, rest @ ..] if action == "start" => {
            parse_session_start_command(conversation_id, rest)
        }
        [action] if action == "list" => Command::SessionList {
            conversation_id: None,
        },
        [action, conversation_id] if action == "list" => Command::SessionList {
            conversation_id: Some(ConversationId::new(conversation_id.as_str())),
        },
        [action, session_id] if action == "status" => Command::SessionStatus {
            session_id: SessionId::new(session_id.as_str()),
        },
        [action, session_id] if action == "events" => Command::SessionEvents {
            session_id: SessionId::new(session_id.as_str()),
        },
        [action, session_id] if action == "approvals" => Command::SessionApprovals {
            session_id: SessionId::new(session_id.as_str()),
        },
        [action, session_id, tool_call_id] if action == "approve" => Command::SessionApprove {
            session_id: SessionId::new(session_id.as_str()),
            tool_call_id: ToolCallId::new(tool_call_id.as_str()),
        },
        [action, session_id, tool_call_id] if action == "deny" => Command::SessionDeny {
            session_id: SessionId::new(session_id.as_str()),
            tool_call_id: ToolCallId::new(tool_call_id.as_str()),
        },
        [action, session_id] if action == "stop" => Command::SessionStop {
            session_id: SessionId::new(session_id.as_str()),
        },
        _ => Command::Invalid,
    }
}

/// Parses `windie run start <conversation_id> [--head <message_id>] [--model <provider/model>]`.
fn parse_session_start_command(conversation_id: &str, args: &[String]) -> Command {
    let mut head_message_id = None;
    let mut model = None;
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
            Some("--model") => {
                let Some(value) = args.get(index + 1) else {
                    return Command::Invalid;
                };
                if model.is_some() {
                    return Command::Invalid;
                }
                model = Some(ModelName::new(value.as_str()));
                index += 2;
            }
            _ => return Command::Invalid,
        }
    }

    Command::SessionStart {
        conversation_id: ConversationId::new(conversation_id),
        head_message_id,
        model,
    }
}
