//! Provider-key environment command parsing.

use super::*;

/// Parses `windie env` subcommands.
pub(super) fn parse_env_command(args: &[String]) -> Command {
    match args {
        [] => Command::Invalid,
        [arg] if arg == "list" => Command::Env(EnvCommand::List),
        [arg] if arg == "path" => Command::Env(EnvCommand::Path),
        [arg, keys @ ..] if arg == "unset" && !keys.is_empty() => {
            Command::Env(EnvCommand::Unset(keys.to_vec()))
        }
        assignments if assignments.iter().all(|arg| arg.contains('=')) => {
            let values = assignments
                .iter()
                .filter_map(|assignment| {
                    let (key, value) = assignment.split_once('=')?;
                    Some((key.to_string(), value.to_string()))
                })
                .collect::<Vec<_>>();
            Command::Env(EnvCommand::Set(values))
        }
        _ => Command::Invalid,
    }
}
