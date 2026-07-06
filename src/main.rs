//! Windie CLI entrypoint.
//!
//! This file wires startup commands to the runtime components. It should stay
//! small and avoid owning business logic, persistence, HTTP, or terminal
//! formatting details.

mod api;
mod cli;
mod context;
mod conversation;
mod error;
mod gateway;
mod image_input;
mod llm;
mod operation;
mod output;
mod perf;
mod policy;
mod runtime;
mod shell;
mod store;
mod tool;
mod tool_catalog;

use anyhow::Result;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::cli::{Command, InsertPart};
use crate::conversation::{
    ConversationId, MessageId, Role, ToolCallId, ToolSchema, ToolSchemaName,
};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, ModelName};
use crate::operation::MessageInputPart;
use crate::output::TerminalOutput;
use crate::perf::{BenchmarkMode, BenchmarkOptions};
use crate::store::Store;
use crate::tool_catalog::available_tool_schemas;

const BASE_URL: &str = "http://localhost:8080/v1";
const GATEWAY_URL: &str = "http://localhost:8080";
const API_ADDRESS: &str = "127.0.0.1:8787";
const MODEL: &str = "openai/gpt-4o-mini";
const INVALID_USAGE_EXIT_CODE: i32 = 2;

/// Process entrypoint. It only dispatches the parsed command to the matching
/// command handler.
#[tokio::main]
async fn main() -> Result<()> {
    match cli::read() {
        Command::Api => api().await,
        Command::Noop => Ok(()),
        Command::Activate {
            conversation_id,
            message_id,
        } => activate_message(conversation_id, message_id),
        Command::Approvals { conversation_id } => list_approvals(conversation_id),
        Command::ApproveTool {
            conversation_id,
            tool_call_id,
        } => approve_tool(conversation_id, tool_call_id).await,
        Command::Help => print_help(),
        Command::Invalid => invalid_usage(),
        Command::Version => print_version(),
        Command::Bench {
            mode,
            conversation_id,
            options,
        } => benchmark(mode, conversation_id, options).await,
        Command::BenchCompare {
            baseline_path,
            current_path,
        } => compare_benchmarks(baseline_path, current_path),
        Command::GatewayStart => start_gateway().await,
        Command::GatewayStop => stop_gateway().await,
        Command::InsertMessage {
            conversation_id,
            role,
            parts,
        } => insert_message(conversation_id, role, &parts),
        Command::InsertToolSchema {
            conversation_id,
            tool_schema,
        } => insert_tool_schema(conversation_id, &tool_schema),
        Command::Inspect {
            conversation_id,
            model,
        } => inspect_conversation(conversation_id, model),
        Command::Tools => list_tools(),
        Command::Fork {
            conversation_id,
            message_id,
        } => fork_conversation(conversation_id, message_id),
        Command::List { json } => list_conversations(json),
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
        Command::RemoveSystemPrompt(conversation_id) => remove_system_prompt(conversation_id),
        Command::RemoveToolSchema {
            conversation_id,
            name,
        } => remove_tool_schema(conversation_id, name),
        Command::DenyTool {
            conversation_id,
            tool_call_id,
        } => deny_tool(conversation_id, tool_call_id),
        Command::Show(conversation_id) => show_conversation(conversation_id),
        Command::Status => status().await,
        Command::SetSystemPrompt {
            conversation_id,
            text,
        } => set_system_prompt(conversation_id, &text),
        Command::Truncate {
            conversation_id,
            message_id,
        } => truncate_conversation(conversation_id, message_id),
        Command::Tree(conversation_id) => show_tree(conversation_id),
        Command::UpdateMessage {
            conversation_id,
            message_id,
            text,
        } => update_message(conversation_id, message_id, &text),
        Command::UpdateToolSchema {
            conversation_id,
            current_name,
            tool_schema,
        } => update_tool_schema(conversation_id, current_name, &tool_schema),
    }
}

/// Starts Windie's local developer API server.
async fn api() -> Result<()> {
    api::serve(api_address(), GATEWAY_URL, BASE_URL, MODEL).await
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
async fn benchmark(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    options: BenchmarkOptions,
) -> Result<()> {
    let output = TerminalOutput;

    if options.runs == 1 && !options.json {
        let baseline = perf::run(
            mode,
            conversation_id,
            gateway_url(),
            base_url(),
            model_name(),
        )
        .await?;

        output.performance_baseline(&baseline);

        return Ok(());
    }

    let report = perf::run_report(
        mode,
        conversation_id,
        gateway_url(),
        base_url(),
        model_name(),
        options.runs,
    )
    .await?;

    if options.json {
        output.performance_report_json(&report)?;
    } else {
        output.performance_report(&report);
    }

    Ok(())
}

/// Reads two JSON benchmark artifacts and prints their median differences.
fn compare_benchmarks(baseline_path: PathBuf, current_path: PathBuf) -> Result<()> {
    let output = TerminalOutput;
    let baseline = perf::read_report(&baseline_path)?;
    let current = perf::read_report(&current_path)?;
    let comparison = perf::compare_reports(&baseline, &current);

    output.performance_comparison(&comparison);

    Ok(())
}

/// Creates an empty persisted conversation and prints only its ID.
fn new_conversation() -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let conversation_id = operation::create_conversation(&store)?;

    output.created_conversation(&conversation_id);

    Ok(())
}

/// Lists persisted conversations without loading their full message history.
fn list_conversations(json: bool) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let conversations = operation::list_conversations(&store)?;

    if json {
        output.conversations_json(&conversations)?;
    } else {
        output.conversations(&conversations);
    }

    Ok(())
}

/// Loads and prints the active path for one conversation.
fn show_conversation(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let messages = operation::active_path(&store, &conversation_id)?;

    output.conversation_messages(&messages);

    Ok(())
}

/// Loads and prints the full message tree for one conversation.
fn show_tree(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let tree = operation::conversation_tree(&store, &conversation_id)?;

    output.conversation_tree(&tree.messages, tree.active_message_id.as_ref());

    Ok(())
}

/// Loads full read-only runtime state and prints it as stable JSON.
///
/// This is the machine-facing inspection path for developer tools. It mirrors
/// the data used by query execution without sending a provider request or
/// mutating the conversation.
fn inspect_conversation(conversation_id: ConversationId, model: Option<ModelName>) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let effective_model = model.unwrap_or_else(model_name);
    let report = operation::inspect_conversation(&store, &conversation_id, &effective_model)?;

    output.inspection_report_json(&report)
}

/// Lists Windie's built-in tool schemas without mutating any conversation.
fn list_tools() -> Result<()> {
    let output = TerminalOutput;
    let tool_schemas = available_tool_schemas();

    output.available_tool_schemas(&tool_schemas);

    Ok(())
}

/// Inserts one explicit message into a conversation.
///
/// The parent is set to the active message so the store keeps a tree and the
/// runtime continues from the selected path.
fn insert_message(conversation_id: ConversationId, role: Role, parts: &[InsertPart]) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let input_parts = message_input_parts(parts);
    let message_id = operation::insert_message(&mut store, &conversation_id, role, &input_parts)?;

    output.inserted_message(&message_id);

    Ok(())
}

/// Converts parsed CLI insert parts into the shared operation input shape.
fn message_input_parts(parts: &[InsertPart]) -> Vec<MessageInputPart> {
    parts
        .iter()
        .map(|part| match part {
            InsertPart::Text(text) => MessageInputPart::Text(text.clone()),
            InsertPart::Image(path) => MessageInputPart::Image(path.clone()),
        })
        .collect()
}

/// Selects one message as the active runtime node.
fn activate_message(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::activate_message(&mut store, &conversation_id, &message_id)?;
    output.activated_message(&message_id);

    Ok(())
}

/// Replaces one message's text without querying the model.
fn update_message(
    conversation_id: ConversationId,
    message_id: MessageId,
    text: &str,
) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::update_message(&mut store, &conversation_id, &message_id, text)?;
    output.updated_message(&message_id);

    Ok(())
}

/// Sets or replaces the conversation-level system prompt.
fn set_system_prompt(conversation_id: ConversationId, text: &str) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::set_system_prompt(&mut store, &conversation_id, text)?;
    output.set_system_prompt(&conversation_id);

    Ok(())
}

/// Clears the conversation-level system prompt.
fn remove_system_prompt(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::remove_system_prompt(&mut store, &conversation_id)?;
    output.removed_system_prompt(&conversation_id);

    Ok(())
}

/// Inserts one conversation-level tool schema.
fn insert_tool_schema(conversation_id: ConversationId, tool_schema: &ToolSchema) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::insert_tool_schema(&mut store, &conversation_id, tool_schema)?;
    output.inserted_tool_schema(&tool_schema.name);

    Ok(())
}

/// Updates one conversation-level tool schema.
fn update_tool_schema(
    conversation_id: ConversationId,
    current_name: ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::update_tool_schema(&mut store, &conversation_id, &current_name, tool_schema)?;
    output.updated_tool_schema(&tool_schema.name);

    Ok(())
}

/// Removes one conversation-level tool schema.
fn remove_tool_schema(conversation_id: ConversationId, name: ToolSchemaName) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::remove_tool_schema(&mut store, &conversation_id, &name)?;
    output.removed_tool_schema(&name);

    Ok(())
}

/// Lists pending tool calls that require explicit user approval.
fn list_approvals(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let approvals = operation::list_tool_approvals(&store, &conversation_id)?;

    output.tool_approvals(&approvals);

    Ok(())
}

/// Executes one approved tool call and stores its tool-result message.
async fn approve_tool(conversation_id: ConversationId, tool_call_id: ToolCallId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let result = operation::approve_tool(&mut store, &conversation_id, &tool_call_id).await?;

    output.tool_execution_result(&result);

    Ok(())
}

/// Stores a rejected tool-result message for one pending tool call.
fn deny_tool(conversation_id: ConversationId, tool_call_id: ToolCallId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let result = operation::deny_tool(&mut store, &conversation_id, &tool_call_id)?;

    output.tool_execution_result(&result);

    Ok(())
}

/// Deletes one conversation and all persisted data owned by it.
fn remove_conversation(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::remove_conversation(&mut store, &conversation_id)?;
    output.removed_conversation(&conversation_id);

    Ok(())
}

/// Deletes one message while preserving the remaining conversation chain.
fn remove_message(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::remove_message(&mut store, &conversation_id, &message_id)?;
    output.removed_message(&message_id);

    Ok(())
}

/// Prunes descendant messages after a checkpoint message inside one conversation.
fn truncate_conversation(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::truncate_conversation(&mut store, &conversation_id, &message_id)?;
    output.truncated_conversation(&conversation_id, &message_id);

    Ok(())
}

/// Creates a new conversation copied through one checkpoint message.
fn fork_conversation(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let forked_conversation_id =
        operation::fork_conversation(&mut store, &conversation_id, &message_id)?;

    output.forked_conversation(&forked_conversation_id);

    Ok(())
}

/// Runs one model response for an existing conversation.
///
/// This is intentionally one runtime turn. If the assistant requests a tool,
/// the CLI prints the stored tool call and exits; users then compose the next
/// steps with `windie approvals`, `windie approve` or `windie deny`, and another
/// `windie query`.
async fn query(conversation_id: ConversationId, model: Option<ModelName>) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::query_conversation(
        &output,
        &mut store,
        &conversation_id,
        gateway_url(),
        base_url(),
        model.unwrap_or_else(model_name),
    )
    .await?;

    Ok(())
}

/// Prints current local runtime readiness.
async fn status() -> Result<()> {
    let output = TerminalOutput;

    output.status(operation::gateway_status(gateway_url()).await);

    Ok(())
}

/// Starts the local Bifrost gateway when it is not already running.
async fn start_gateway() -> Result<()> {
    let output = TerminalOutput;
    let status = operation::start_gateway(gateway_url()).await?;

    match status {
        crate::gateway::GatewayStart::AlreadyRunning => output.gateway_already_running(),
        crate::gateway::GatewayStart::Started => output.gateway_started(),
    }

    Ok(())
}

/// Stops the local Bifrost gateway process owned by the configured port.
async fn stop_gateway() -> Result<()> {
    let output = TerminalOutput;
    let status = operation::stop_gateway(gateway_url()).await?;

    match status {
        crate::gateway::GatewayStop::NotRunning => output.gateway_not_running(),
        crate::gateway::GatewayStop::Stopped => output.gateway_stopped(),
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

/// Centralizes the local developer API bind address.
fn api_address() -> SocketAddr {
    API_ADDRESS
        .parse()
        .expect("hardcoded API address must be valid")
}
