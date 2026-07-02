use std::env;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum Command {
    Chat,
    Help,
    List,
    New,
    Open(String),
    Version,
}

pub fn read() -> Command {
    command_from_args(env::args())
}

pub fn print_help() {
    println!("windie");
    println!();
    println!("Usage:");
    println!("  windie");
    println!("  windie new");
    println!("  windie list");
    println!("  windie open <conversation_id>");
    println!();
    println!("Options:");
    println!("  -h, --help       Show help");
    println!("  -V, --version    Show version");
}

pub fn print_version() {
    println!("windie {VERSION}");
}

fn command_from_args(args: impl IntoIterator<Item = String>) -> Command {
    let mut args = args.into_iter();
    let _program = args.next();

    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Command::Help;
        }

        if arg == "--version" || arg == "-V" {
            return Command::Version;
        }

        if arg == "new" {
            return Command::New;
        }

        if arg == "list" {
            return Command::List;
        }

        if arg == "open" {
            if let Some(conversation_id) = args.next() {
                return Command::Open(conversation_id);
            }

            return Command::Help;
        }
    }

    Command::Chat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_chat_command_by_default() {
        let command = command_from_args(["windie".to_string()]);

        assert!(matches!(command, Command::Chat));
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
    fn reads_first_known_command() {
        let command = command_from_args([
            "windie".to_string(),
            "--version".to_string(),
            "--help".to_string(),
        ]);

        assert!(matches!(command, Command::Version));
    }

    #[test]
    fn reads_new_command() {
        let command = command_from_args(["windie".to_string(), "new".to_string()]);

        assert!(matches!(command, Command::New));
    }

    #[test]
    fn reads_list_command() {
        let command = command_from_args(["windie".to_string(), "list".to_string()]);

        assert!(matches!(command, Command::List));
    }

    #[test]
    fn reads_open_command() {
        let command = command_from_args([
            "windie".to_string(),
            "open".to_string(),
            "conversation-id".to_string(),
        ]);

        assert!(matches!(command, Command::Open(id) if id == "conversation-id"));
    }

    #[test]
    fn reads_open_without_id_as_help() {
        let command = command_from_args(["windie".to_string(), "open".to_string()]);

        assert!(matches!(command, Command::Help));
    }
}
