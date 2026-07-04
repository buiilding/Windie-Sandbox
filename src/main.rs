//! Windie CLI entrypoint.
//!
//! This file wires startup commands to the runtime components. It should stay
//! small and avoid owning business logic, persistence, HTTP, or terminal
//! formatting details.

mod cli;
mod context;
mod conversation;
mod gateway;
mod image_input;
mod llm;
mod output;
mod perf;
mod runtime;
mod store;

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli::{Command, InsertPart};
use crate::context::{ContextBuilder, ContextParts};
use crate::conversation::{ConversationId, MessageId, Role, ToolSchema, ToolSchemaName};
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::image_input::read_image_input;
use crate::llm::{BaseUrl, BifrostClient, ModelName};
use crate::output::{InspectionReport, TerminalOutput};
use crate::perf::{BenchmarkMode, BenchmarkOptions};
use crate::runtime::query_conversation;
use crate::store::{ImagePayload, MessagePayload, Store};

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
        Command::Activate {
            conversation_id,
            message_id,
        } => activate_message(conversation_id, message_id),
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
        .await
        .context("failed to run performance baseline")?;

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
    .await
    .context("failed to run performance report")?;

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
    let baseline =
        perf::read_report(&baseline_path).context("failed to read baseline benchmark report")?;
    let current =
        perf::read_report(&current_path).context("failed to read current benchmark report")?;
    let comparison = perf::compare_reports(&baseline, &current);

    output.performance_comparison(&comparison);

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
fn list_conversations(json: bool) -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let conversations = store
        .list_conversations()
        .context("failed to list conversations")?;

    if json {
        output.conversations_json(&conversations)?;
    } else {
        output.conversations(&conversations);
    }

    Ok(())
}

/// Loads and prints the active path for one conversation.
fn show_conversation(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let messages = store
        .load_active_path(&conversation_id)
        .with_context(|| format!("failed to show conversation {conversation_id}"))?;

    output.conversation_messages(&messages);

    Ok(())
}

/// Loads and prints the full message tree for one conversation.
fn show_tree(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let messages = store
        .load_message_tree(&conversation_id)
        .with_context(|| format!("failed to show conversation tree {conversation_id}"))?;
    let active_message_id = store
        .active_message_id(&conversation_id)
        .context("failed to load active message")?;

    output.conversation_tree(&messages, active_message_id.as_ref());

    Ok(())
}

/// Loads full read-only runtime state and prints it as stable JSON.
///
/// This is the machine-facing inspection path for developer tools. It mirrors
/// the data used by query execution without sending a provider request or
/// mutating the conversation.
fn inspect_conversation(conversation_id: ConversationId, model: Option<ModelName>) -> Result<()> {
    let store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let effective_model = model.unwrap_or_else(model_name);
    let active_message_id = store
        .active_message_id(&conversation_id)
        .context("failed to load active message")?;
    let messages = store
        .load_message_tree(&conversation_id)
        .with_context(|| format!("failed to inspect conversation tree {conversation_id}"))?;
    let tool_schemas = store
        .load_tool_schemas(&conversation_id)
        .context("failed to load tool schemas")?;
    let context_parts = ContextBuilder::load_parts(&store, &conversation_id)
        .context("failed to load model context parts")?;
    let model_context = ContextBuilder::flatten(ContextParts {
        active_path: context_parts.active_path.clone(),
        system_prompt: context_parts.system_prompt.clone(),
        compaction: context_parts.compaction.clone(),
    });
    let report = InspectionReport::new(
        &conversation_id,
        active_message_id.as_ref(),
        effective_model.as_str(),
        context_parts.system_prompt,
        tool_schemas,
        messages,
        context_parts.active_path,
        model_context,
        context_parts.compaction,
    );

    output.inspection_report_json(&report)
}

/// Inserts one explicit message into a conversation.
///
/// The parent is set to the active message so the store keeps a tree and the
/// runtime continues from the selected path.
fn insert_message(conversation_id: ConversationId, role: Role, parts: &[InsertPart]) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;
    let parent_message_id = store
        .active_message_id(&conversation_id)
        .context("failed to load active message")?;
    let has_image = parts
        .iter()
        .any(|part| matches!(part, InsertPart::Image(_)));
    let has_multiple_parts = parts.len() > 1;
    let content = insert_content(parts);
    let message_id = if has_image || has_multiple_parts {
        if role != Role::User {
            anyhow::bail!("multi-part input is only supported for user messages");
        }

        let loaded_parts = parts
            .iter()
            .map(load_insert_part)
            .collect::<Result<Vec<_>>>()?;
        let payloads = loaded_parts
            .iter()
            .map(|part| match part {
                LoadedInsertPart::Text(text) => MessagePayload::Text(text),
                LoadedInsertPart::Image(image) => MessagePayload::Image(ImagePayload {
                    mime_type: &image.mime_type,
                    bytes: &image.bytes,
                }),
            })
            .collect::<Vec<_>>();
        store
            .insert_user_message_with_parts(
                &conversation_id,
                parent_message_id.as_ref(),
                &content,
                &payloads,
            )
            .context("failed to insert multi-part message")?
    } else {
        store
            .insert_message(
                &conversation_id,
                parent_message_id.as_ref(),
                role,
                &content,
                None,
            )
            .context("failed to insert message")?
    };

    output.inserted_message(&message_id);

    Ok(())
}

/// Loaded version of one ordered insert part.
enum LoadedInsertPart {
    Text(String),
    Image(crate::image_input::ImageInput),
}

/// Reads file-backed insert parts while preserving text parts as provided.
fn load_insert_part(part: &InsertPart) -> Result<LoadedInsertPart> {
    match part {
        InsertPart::Text(text) => Ok(LoadedInsertPart::Text(text.clone())),
        InsertPart::Image(path) => read_image_input(path).map(LoadedInsertPart::Image),
    }
}

/// Builds the plain text preview stored in the message row.
fn insert_content(parts: &[InsertPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            InsertPart::Text(text) => Some(text.as_str()),
            InsertPart::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Selects one message as the active runtime node.
fn activate_message(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .set_active_message(&conversation_id, &message_id)
        .context("failed to activate message")?;
    output.activated_message(&message_id);

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

/// Sets or replaces the conversation-level system prompt.
fn set_system_prompt(conversation_id: ConversationId, text: &str) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .set_system_prompt(&conversation_id, text)
        .context("failed to set system prompt")?;
    output.set_system_prompt(&conversation_id);

    Ok(())
}

/// Clears the conversation-level system prompt.
fn remove_system_prompt(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .remove_system_prompt(&conversation_id)
        .context("failed to remove system prompt")?;
    output.removed_system_prompt(&conversation_id);

    Ok(())
}

/// Inserts one conversation-level tool schema.
fn insert_tool_schema(conversation_id: ConversationId, tool_schema: &ToolSchema) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .insert_tool_schema(&conversation_id, tool_schema)
        .context("failed to insert tool schema")?;
    output.inserted_tool_schema(&tool_schema.name);

    Ok(())
}

/// Updates one conversation-level tool schema.
fn update_tool_schema(
    conversation_id: ConversationId,
    current_name: ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .update_tool_schema(&conversation_id, &current_name, tool_schema)
        .context("failed to update tool schema")?;
    output.updated_tool_schema(&tool_schema.name);

    Ok(())
}

/// Removes one conversation-level tool schema.
fn remove_tool_schema(conversation_id: ConversationId, name: ToolSchemaName) -> Result<()> {
    let mut store = Store::open().context("failed to open store")?;
    let output = TerminalOutput;

    store
        .remove_tool_schema(&conversation_id, &name)
        .context("failed to remove tool schema")?;
    output.removed_tool_schema(&name);

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

/// Prunes descendant messages after a checkpoint message inside one conversation.
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
