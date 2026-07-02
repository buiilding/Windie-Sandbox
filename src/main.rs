mod cli;
mod conversation;
mod gateway;
mod input;
mod llm;
mod output;
mod runtime;
mod store;

use anyhow::Result;

use crate::cli::Command;
use crate::gateway::BifrostGateway;
use crate::input::TerminalInput;
use crate::llm::BifrostClient;
use crate::output::TerminalOutput;
use crate::runtime::ChatRuntime;
use crate::store::{ConversationInfo, Store};

const BASE_URL: &str = "http://localhost:8080/v1";
const GATEWAY_URL: &str = "http://localhost:8080";
const MODEL: &str = "openai/gpt-4o-mini";

#[tokio::main]
async fn main() -> Result<()> {
    match cli::read() {
        Command::Help => {
            cli::print_help();
            Ok(())
        }
        Command::Version => {
            cli::print_version();
            Ok(())
        }
        Command::List => list_conversations(),
        Command::New => new_conversation().await,
        Command::Open(conversation_id) => open_conversation(conversation_id).await,
        Command::Chat => continue_active_conversation().await,
    }
}

async fn continue_active_conversation() -> Result<()> {
    let store = Store::open()?;
    let conversation_id = store.get_or_create_active_conversation()?;

    run_chat(store, conversation_id).await
}

async fn new_conversation() -> Result<()> {
    let store = Store::open()?;
    let conversation_id = store.create_conversation()?;
    store.set_active_conversation(&conversation_id)?;

    run_chat(store, conversation_id).await
}

async fn open_conversation(conversation_id: String) -> Result<()> {
    let store = Store::open()?;
    store.set_active_conversation(&conversation_id)?;

    run_chat(store, conversation_id).await
}

fn list_conversations() -> Result<()> {
    let store = Store::open()?;
    let active_conversation_id = store.active_conversation_id()?;
    let conversations = store.list_conversations()?;

    print_conversations(&conversations, active_conversation_id.as_deref());

    Ok(())
}

fn print_conversations(conversations: &[ConversationInfo], active_conversation_id: Option<&str>) {
    if conversations.is_empty() {
        println!("no conversations");
        return;
    }

    println!("conversations");
    for conversation in conversations {
        let marker = if Some(conversation.id.as_str()) == active_conversation_id {
            "*"
        } else {
            " "
        };
        let title = conversation.title.as_deref().unwrap_or("");

        if title.is_empty() {
            println!(
                "{marker} {}  {}",
                conversation.id,
                message_count(conversation.message_count)
            );
        } else {
            println!(
                "{marker} {}  {}  {}",
                conversation.id,
                message_count(conversation.message_count),
                title
            );
        }
    }
}

fn message_count(count: i64) -> String {
    if count == 1 {
        "1 message".to_string()
    } else {
        format!("{count} messages")
    }
}

async fn run_chat(store: Store, conversation_id: String) -> Result<()> {
    let gateway = BifrostGateway::new(GATEWAY_URL);
    gateway.ensure_running().await?;

    let input = TerminalInput::new()?;
    let output = TerminalOutput;
    let llm = BifrostClient::new(BASE_URL, MODEL);
    let messages = store.load_messages(&conversation_id)?;
    let mut runtime = ChatRuntime::new(input, output, llm, store, conversation_id, messages);

    runtime.run().await
}
