//! Windie CLI entrypoint.
//!
//! This file wires startup commands to the runtime components. It should stay
//! small and avoid owning business logic, persistence, HTTP, or terminal
//! formatting details.

mod cli;
mod context;
mod conversation;
mod gateway;
mod llm;
mod output;
mod perf;
mod runtime;
mod store;

use anyhow::{Context, Result};

use crate::cli::Command;
use crate::conversation::{ConversationId, MessageId, Role};
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::llm::{BaseUrl, BifrostClient, ModelName};
use crate::output::TerminalOutput;
use crate::perf::BenchmarkMode;
use crate::runtime::query_conversation;
use crate::store::Store;

const BASE_URL: &str = "http://localhost:8080/v1";
const GATEWAY_URL: &str = "http://localhost:8080";
const MODEL: &str = "openai/gpt-4o-mini";
const INVALID_USAGE_EXIT_CODE: i32 = 2;

#[tokio::main]
async fn main() -> Result<()> {
    match cli::read() {
        Command::Noop => Ok(()),
        Command::Help => print_help(),
        Command::Invalid => invalid_usage(),
        Command::Version => print_version(),
        Command::Bench {
            mode,
            conversation_id,
        } => benchmark(mode, conversation_id).await,
        Command::GatewayStart => start_gateway().await,
        Command::GatewayStop => stop_gateway().await,
        Command::Append {
            conversation_id,
            role,
            text,
        } => append_message(conversation_id, role, &text),
        Command::List => list_conversations(),
        Command::New => new_conversation(),
        Command::Query {
            conversation_id,
            model,
        } => query(conversation_id, model).await,
        Command::RemoveConversation(conversation_id) => remove_conversation(conversation_id),
        Command::RemoveMessage {
            conversation_id,
            message_id,
        } => remove_message(conversation_id, message_id),
        Command::Show(conversation_id) => show_conversation(conversation_id),
        Command::Status => status().await,
        Command::Update {
            conversation_id,
            message_id,
            text,
        } => update_message(conversation_id, message_id, &text),
    }
}

fn print_help() -> Result<()> {
    let output = TerminalOutput;
    output.help();

    Ok(())
}

fn invalid_usage() -> Result<()> {
    let output = TerminalOutput;
    output.invalid_usage();
    std::process::exit(INVALID_USAGE_EXIT_CODE);
}

fn print_version() -> Result<()> {
    let output = TerminalOutput;
    output.version();

    Ok(())
}

async fn benchmark(mode: BenchmarkMode, conversation_id: Option<ConversationId>) -> Result<()> {
    let output = TerminalOutput;
    let baseline = perf::run(
        mode,
        conversation_id,
        gateway_url(),
        base_url(),
        model_name(),
    )
    .await
    .context("failed to run performance baseline")?;

    output.performance_baseline(&baseline);

    Ok(())
}

fn new_conversation() -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let conversation_id = store
        .create_conversation()
        .context("failed to create conversation")?;

    output.created_conversation(&conversation_id);

    Ok(())
}

fn list_conversations() -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let conversations = store
        .list_conversations()
        .context("failed to list conversations")?;

    output.conversations(&conversations);

    Ok(())
}

fn show_conversation(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let messages = store
        .load_messages(&conversation_id)
        .with_context(|| format!("failed to show conversation {conversation_id}"))?;

    output.conversation_messages(&messages);

    Ok(())
}

fn append_message(conversation_id: ConversationId, role: Role, text: &str) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let parent_message_id = store
        .load_messages(&conversation_id)
        .context("failed to load conversation messages")?
        .last()
        .and_then(|message| message.id.clone());
    let message_id = store
        .save_message(
            &conversation_id,
            parent_message_id.as_ref(),
            role,
            text,
            None,
        )
        .context("failed to append message")?;

    output.appended_message(&message_id);

    Ok(())
}

fn update_message(
    conversation_id: ConversationId,
    message_id: MessageId,
    text: &str,
) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .update_message_text(&conversation_id, &message_id, text)
        .context("failed to update message")?;
    output.updated_message(&message_id);

    Ok(())
}

fn remove_conversation(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .delete_conversation(&conversation_id)
        .context("failed to remove conversation")?;
    output.removed_conversation(&conversation_id);

    Ok(())
}

fn remove_message(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .delete_message(&conversation_id, &message_id)
        .context("failed to remove message")?;
    output.removed_message(&message_id);

    Ok(())
}

async fn query(conversation_id: ConversationId, model: Option<ModelName>) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let gateway = BifrostGateway::new(gateway_url());
    gateway
        .require_running()
        .await
        .context("failed to prepare Bifrost gateway")?;
    let llm = BifrostClient::new(base_url(), model.unwrap_or_else(model_name));

    query_conversation(&output, &llm, &mut store, &conversation_id)
        .await
        .context("failed to query conversation")?;

    Ok(())
}

async fn status() -> Result<()> {
    let output = TerminalOutput;
    let gateway = BifrostGateway::new(gateway_url());

    output.status(gateway.is_running().await);

    Ok(())
}

async fn start_gateway() -> Result<()> {
    let output = TerminalOutput;
    let gateway = BifrostGateway::new(gateway_url());
    let status = gateway.start().await.context("failed to start gateway")?;

    match status {
        GatewayStart::AlreadyRunning => output.gateway_already_running(),
        GatewayStart::Started => output.gateway_started(),
    }

    Ok(())
}

async fn stop_gateway() -> Result<()> {
    let output = TerminalOutput;
    let gateway = BifrostGateway::new(gateway_url());
    let status = gateway.stop().await.context("failed to stop gateway")?;

    match status {
        GatewayStop::NotRunning => output.gateway_not_running(),
        GatewayStop::Stopped => output.gateway_stopped(),
    }

    Ok(())
}

fn gateway_url() -> GatewayUrl {
    GatewayUrl::new(GATEWAY_URL)
}

fn base_url() -> BaseUrl {
    BaseUrl::new(BASE_URL)
}

fn model_name() -> ModelName {
    ModelName::new(MODEL)
}
