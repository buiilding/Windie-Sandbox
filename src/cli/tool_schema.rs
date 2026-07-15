//! Tool schema command parsing.

use super::*;

/// Parses `windie insert <conversation_id> toolschema --name <name> --description <text> --parameters <json>`.
pub(super) fn parse_insert_tool_schema_command(conversation_id: &str, args: &[String]) -> Command {
    let Some(tool_schema) = parse_tool_schema_flags(args) else {
        return Command::Invalid;
    };

    Command::InsertToolSchema {
        conversation_id: ConversationId::new(conversation_id),
        tool_schema,
    }
}

/// Parses a complete tool schema flag set.
pub(super) fn parse_tool_schema_flags(args: &[String]) -> Option<ToolSchema> {
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
