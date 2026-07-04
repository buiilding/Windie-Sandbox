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

/// Process entrypoint. It only dispatches the parsed command to the matching
/// command handler.
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
        Command::Fork {
            conversation_id,
            message_id,
        } => fork_conversation(conversation_id, message_id),
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
        Command::Truncate {
            conversation_id,
            message_id,
        } => truncate_conversation(conversation_id, message_id),
        Command::Update {
            conversation_id,
            message_id,
            text,
        } => update_message(conversation_id, message_id, &text),
    }
}

/// Prints the generated CLI help text.
fn print_help() -> Result<()> {
    let output = TerminalOutput;
    output.help();

    Ok(())
}

/// Prints usage and exits with code 2, the conventional CLI code for bad
/// command usage.
fn invalid_usage() -> Result<()> {
    let output = TerminalOutput;
    output.invalid_usage();
    std::process::exit(INVALID_USAGE_EXIT_CODE);
}

/// Prints the package version embedded by Cargo.
fn print_version() -> Result<()> {
    let output = TerminalOutput;
    output.version();

    Ok(())
}

/// Runs one benchmark mode and sends the measured baseline to the output
/// boundary.
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

/// Creates an empty persisted conversation and prints only its ID.
fn new_conversation() -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let conversation_id = store
        .create_conversation()
        .context("failed to create conversation")?;

    output.created_conversation(&conversation_id);

    Ok(())
}

/// Lists persisted conversations without loading their full message history.
fn list_conversations() -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let conversations = store
        .list_conversations()
        .context("failed to list conversations")?;

    output.conversations(&conversations);

    Ok(())
}

/// Loads and prints the messages for one conversation.
fn show_conversation(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let messages = store
        .load_messages(&conversation_id)
        .with_context(|| format!("failed to show conversation {conversation_id}"))?;

    output.conversation_messages(&messages);

    Ok(())
}

/// Appends one explicit message to a conversation.
///
/// The parent is set to the current last message so the store keeps a simple
/// message chain for future editing/forking behavior.
fn append_message(conversation_id: ConversationId, role: Role, text: &str) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let parent_message_id = store
        .load_messages(&conversation_id)
        .context("failed to load conversation messages")?
        .last()
        .and_then(|message| message.id.clone());
    let message_id = store
        .append_message(
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

/// Replaces one message's text without querying the model.
fn update_message(
    conversation_id: ConversationId,
    message_id: MessageId,
    text: &str,
) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .replace_message(&conversation_id, &message_id, text)
        .context("failed to update message")?;
    output.updated_message(&message_id);

    Ok(())
}

/// Deletes one conversation and all persisted data owned by it.
fn remove_conversation(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .remove_conversation(&conversation_id)
        .context("failed to remove conversation")?;
    output.removed_conversation(&conversation_id);

    Ok(())
}

/// Deletes one message while preserving the remaining conversation chain.
fn remove_message(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .remove_message(&conversation_id, &message_id)
        .context("failed to remove message")?;
    output.removed_message(&message_id);

    Ok(())
}

/// Removes all messages after a checkpoint message inside one conversation.
fn truncate_conversation(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .truncate_after_message(&conversation_id, &message_id)
        .context("failed to truncate conversation")?;
    output.truncated_conversation(&conversation_id, &message_id);

    Ok(())
}

/// Creates a new conversation copied through one checkpoint message.
fn fork_conversation(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let forked_conversation_id = store
        .fork_conversation_at_message(&conversation_id, &message_id)
        .context("failed to fork conversation")?;

    output.forked_conversation(&forked_conversation_id);

    Ok(())
}

/// Runs one model response for an existing conversation.
///
/// This is the CLI handler only. The reusable runtime flow lives in
/// `runtime::query_conversation`.
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

/// Prints current local runtime readiness.
async fn status() -> Result<()> {
    let output = TerminalOutput;
    let gateway = BifrostGateway::new(gateway_url());

    output.status(gateway.is_running().await);

    Ok(())
}

/// Starts the local Bifrost gateway when it is not already running.
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

/// Stops the local Bifrost gateway process owned by the configured port.
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

/// Centralizes the gateway health base URL.
fn gateway_url() -> GatewayUrl {
    GatewayUrl::new(GATEWAY_URL)
}

/// Centralizes the OpenAI-compatible API base URL.
fn base_url() -> BaseUrl {
    BaseUrl::new(BASE_URL)
}

/// Centralizes the default model while config is intentionally not in scope.
fn model_name() -> ModelName {
    ModelName::new(MODEL)
}
