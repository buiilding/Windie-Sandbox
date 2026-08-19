//! Windie CLI entrypoint.
//!
//! This file wires startup commands to the runtime components. It should stay
//! small and avoid owning business logic, persistence, HTTP, or terminal
//! formatting details.

use anyhow::Result;
use std::net::SocketAddr;

use windie::cli::{Command, EnvCommand, InsertPart};
use windie::conversation::{ConversationId, MessageId, Role, ToolCallId};
use windie::llm::gateway::GatewayUrl;
use windie::llm::{BaseUrl, ModelName};
use windie::local::process::ManagedComponent;
use windie::operation::MessageInputPart;
use windie::output::TerminalOutput;
use windie::session::SessionId;
use windie::store::Store;
use windie::tool::ToolProviderRegistry;
use windie::tool::{ProviderToolName, ToolProviderId, ToolSchema, ToolSchemaName};
use windie::{cli, config, local, operation};

const INVALID_USAGE_EXIT_CODE: i32 = 2;

/// Process entrypoint. It only dispatches the parsed command to the matching
/// command handler.
#[tokio::main]
async fn main() -> Result<()> {
    match cli::read() {
        Command::ApiStart => start_api_process(),
        Command::ApiStop => stop_api_process(),
        Command::ApiOutput => output_component(ManagedComponent::Api),
        Command::ApiRun => run_api().await,
        Command::InspectorStart => start_inspector_process(),
        Command::InspectorStop => stop_inspector_process(),
        Command::InspectorOutput => output_component(ManagedComponent::Inspector),
        Command::Onboard => onboard().await,
        Command::AttachTool {
            conversation_id,
            provider_id,
            tool_name,
        } => attach_tool(conversation_id, provider_id, tool_name),
        Command::Help => print_help(),
        Command::Invalid => invalid_usage(),
        Command::Version => print_version(),
        Command::Env(command) => env_command(command),
        Command::Install { target } => install_target(&target),
        Command::Uninstall { yes, dry_run } => uninstall_windie(yes, dry_run).await,
        Command::GatewayStart => start_gateway().await,
        Command::GatewayStop => stop_gateway().await,
        Command::GatewayOutput => output_component(ManagedComponent::Gateway),
        Command::Tray => local::tray::run(),
        Command::InsertMessage {
            conversation_id,
            head_message_id,
            role,
            parts,
        } => insert_message(conversation_id, head_message_id, role, &parts),
        Command::InsertToolSchema {
            conversation_id,
            tool_schema,
        } => insert_tool_schema(conversation_id, &tool_schema),
        Command::Inspect {
            conversation_id,
            head_message_id,
            model,
        } => inspect_conversation(conversation_id, head_message_id, model),
        Command::Tools { provider_id } => list_tools(provider_id),
        Command::Fork {
            conversation_id,
            message_id,
        } => fork_conversation(conversation_id, message_id),
        Command::List { json } => list_conversations(json),
        Command::Models => list_models().await,
        Command::New => new_conversation().await,
        Command::SessionStart {
            conversation_id,
            head_message_id,
            model,
        } => session_start(conversation_id, head_message_id, model).await,
        Command::SessionList { conversation_id } => session_list(conversation_id),
        Command::SessionStatus { session_id } => session_status(session_id),
        Command::SessionEvents { session_id } => session_events(session_id),
        Command::SessionApprovals { session_id } => session_approvals(session_id),
        Command::SessionApprove {
            session_id,
            tool_call_id,
        } => session_approve(session_id, tool_call_id).await,
        Command::SessionDeny {
            session_id,
            tool_call_id,
        } => session_deny(session_id, tool_call_id).await,
        Command::SessionStop { session_id } => session_stop(session_id),
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
        Command::DetachTool {
            conversation_id,
            schema_name,
        } => detach_tool(conversation_id, schema_name),
        Command::Show(conversation_id) => show_conversation(conversation_id),
        Command::Status => status().await,
        Command::SetSystemPrompt {
            conversation_id,
            text,
        } => set_system_prompt(conversation_id, &text),
        Command::SetModel {
            conversation_id,
            model,
        } => set_model(conversation_id, model),
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
async fn run_api() -> Result<()> {
    let gateway_url = gateway_url();
    let base_url = base_url();
    windie::api::serve(api_address(), gateway_url.as_str(), base_url.as_str()).await
}

/// Starts the detached Windie API process.
fn start_api_process() -> Result<()> {
    let report = windie::operation::start_api()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Stops the Windie API process without touching Bifrost.
fn stop_api_process() -> Result<()> {
    let report = windie::operation::stop_api()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Starts the detached Inspector process.
fn start_inspector_process() -> Result<()> {
    let report = windie::operation::start_inspector()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Stops the Inspector process without touching the API or Bifrost.
fn stop_inspector_process() -> Result<()> {
    let report = windie::operation::stop_inspector()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Prints persisted output for one independent local component.
fn output_component(component: ManagedComponent) -> Result<()> {
    let output = windie::operation::component_output(component)?;
    TerminalOutput.component_output(component, &output);
    Ok(())
}

/// Runs the terminal-only onboarding wizard without opening a browser.
async fn onboard() -> Result<()> {
    let mut console = cli::TerminalOnboarding::new();
    let gateway_url = gateway_url();
    operation::run_onboarding(
        &mut console,
        gateway_url.clone(),
        gateway_url.as_str(),
        base_url(),
    )
    .await
}

/// Confirms and runs complete Windie cleanup from the CLI boundary.
async fn uninstall_windie(yes: bool, dry_run: bool) -> Result<()> {
    let output = TerminalOutput;
    if !dry_run && !yes && !confirm_uninstall()? {
        println!("windie uninstall: cancelled");
        return Ok(());
    }

    let report = operation::uninstall_windie(dry_run, gateway_url()).await?;
    output.uninstall_report(&report);
    Ok(())
}

/// Reads the destructive-action confirmation without exposing provider keys.
fn confirm_uninstall() -> Result<bool> {
    use std::io::{self, Write};

    eprintln!(
        "This will stop Windie and delete all Windie data, provider keys, logs, and installed binaries."
    );
    eprint!("Continue? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
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

/// Sessions one user-local environment command.
fn env_command(command: EnvCommand) -> Result<()> {
    let output = TerminalOutput;

    match command {
        EnvCommand::Set(assignments) => {
            let path = local::set_env_values(&assignments)?;
            output.env_updated(&path, assignments.len());
        }
        EnvCommand::List => {
            let keys = local::list_env_keys()?;
            output.env_keys(&keys);
        }
        EnvCommand::Unset(keys) => {
            let path = local::unset_env_values(&keys)?;
            output.env_updated(&path, keys.len());
        }
        EnvCommand::Path => {
            let path = local::env_file_path()?;
            output.env_path(&path);
        }
    }

    Ok(())
}

/// Installs or verifies one approved Windie dependency.
fn install_target(target: &str) -> Result<()> {
    let report = local::install_target(target)?;
    let output = TerminalOutput;
    output.install_report(&report);

    Ok(())
}

/// Creates an empty persisted conversation and prints only its ID.
async fn new_conversation() -> Result<()> {
    let models = operation::list_models(gateway_url(), base_url()).await?;
    let model = operation::preferred_model(models).ok_or_else(|| {
        anyhow::anyhow!("no models are available; configure a provider key first")
    })?;
    let store = Store::open()?;
    let output = TerminalOutput;
    let conversation_id = operation::create_conversation(&store, &model)?;

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

/// Loads and prints all messages for one conversation.
fn show_conversation(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let messages = store.load_messages(&conversation_id)?;

    output.conversation_messages(&messages);

    Ok(())
}

/// Loads and prints the full message tree for one conversation.
fn show_tree(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let tree = operation::conversation_tree(&store, &conversation_id)?;

    output.conversation_tree(&tree.messages);

    Ok(())
}

/// Loads full read-only runtime state and prints it as stable JSON.
///
/// This is the machine-facing inspection path for developer tools. It mirrors
/// the data used by query execution without sending a provider request or
/// mutating the conversation.
fn inspect_conversation(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let report =
        operation::inspect_conversation(&store, &conversation_id, head_message_id.as_ref(), model)?;

    output.inspection_report_json(&report)
}

/// Lists provider tools without mutating any conversation.
fn list_tools(provider_id: Option<ToolProviderId>) -> Result<()> {
    let output = TerminalOutput;
    let tools = provider_id
        .as_ref()
        .map(operation::available_provider_tools)
        .unwrap_or_else(operation::available_tools)?;

    output.available_tools(&tools);

    Ok(())
}

/// Attaches one provider tool to a conversation.
fn attach_tool(
    conversation_id: ConversationId,
    provider_id: ToolProviderId,
    tool_name: ProviderToolName,
) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let schema_name =
        operation::attach_tool(&mut store, &conversation_id, &provider_id, &tool_name)?;

    output.inserted_tool_schema(&schema_name);

    Ok(())
}

/// Inserts one explicit message into a conversation.
fn insert_message(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    role: Role,
    parts: &[InsertPart],
) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let input_parts = message_input_parts(parts);
    let message_id = operation::insert_message(
        &mut store,
        &conversation_id,
        head_message_id.as_ref(),
        role,
        &input_parts,
    )?;

    output.inserted_message(&message_id);

    Ok(())
}

/// Converts parsed CLI insert parts into the shared operation input shape.
fn message_input_parts(parts: &[InsertPart]) -> Vec<MessageInputPart> {
    parts
        .iter()
        .map(|part| match part {
            InsertPart::Text(text) => MessageInputPart::Text(text.clone()),
            InsertPart::Image(path) => MessageInputPart::ImagePath(path.clone()),
        })
        .collect()
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

/// Sets or replaces the root-scoped system prompt.
fn set_system_prompt(conversation_id: ConversationId, text: &str) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::set_system_prompt(&mut store, &conversation_id, text)?;
    output.set_system_prompt(&conversation_id);

    Ok(())
}

/// Clears the root-scoped system prompt.
fn remove_system_prompt(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::remove_system_prompt(&mut store, &conversation_id)?;
    output.removed_system_prompt(&conversation_id);

    Ok(())
}

/// Inserts one root-scoped tool schema.
fn insert_tool_schema(conversation_id: ConversationId, tool_schema: &ToolSchema) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::insert_tool_schema(&mut store, &conversation_id, tool_schema)?;
    output.inserted_tool_schema(&tool_schema.name);

    Ok(())
}

/// Updates one root-scoped tool schema.
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

/// Removes one root-scoped tool schema.
fn remove_tool_schema(conversation_id: ConversationId, name: ToolSchemaName) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::remove_tool_schema(&mut store, &conversation_id, &name)?;
    output.removed_tool_schema(&name);

    Ok(())
}

/// Detaches one provider-backed tool schema from a conversation.
fn detach_tool(conversation_id: ConversationId, schema_name: ToolSchemaName) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::detach_tool(&mut store, &conversation_id, &schema_name)?;
    output.removed_tool_schema(&schema_name);

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

/// Lists models exposed by the currently running Bifrost gateway.
async fn list_models() -> Result<()> {
    let output = TerminalOutput;
    let models = operation::list_models(gateway_url(), base_url()).await?;

    output.models(&models);

    Ok(())
}

/// Starts and advances one session from an explicit or default conversation head.
async fn session_start(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
) -> Result<()> {
    operation::start_cli_session(
        conversation_id,
        head_message_id,
        model,
        gateway_url(),
        base_url(),
    )
    .await
}

/// Lists persisted sessiontime sessions.
fn session_list(conversation_id: Option<ConversationId>) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let sessions = match conversation_id {
        Some(conversation_id) => store.list_conversation_sessions(&conversation_id)?,
        None => store.list_sessions()?,
    };

    output.sessions(&sessions);

    Ok(())
}

/// Prints one persisted session status.
fn session_status(session_id: SessionId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let session = store.load_session(&session_id)?;

    output.session_status(&session);

    Ok(())
}

/// Prints persisted session events.
fn session_events(session_id: SessionId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;

    for event in store.load_session_events_after(&session_id, None)? {
        output.session_event(&event);
    }

    Ok(())
}

/// Lists session-owned approvals for one session.
fn session_approvals(session_id: SessionId) -> Result<()> {
    let store = Store::open()?;
    let output = TerminalOutput;
    let registry = ToolProviderRegistry::with_installed_plugins()?;
    let session = store.load_session(&session_id)?;
    let approvals = operation::list_session_approvals_with_registry(&store, &session, &registry)?;

    output.session_approvals(&approvals);

    Ok(())
}

/// Executes one approved session-owned tool call and continues that session.
async fn session_approve(session_id: SessionId, tool_call_id: ToolCallId) -> Result<()> {
    operation::approve_cli_session_tool(session_id, tool_call_id, gateway_url(), base_url()).await
}

/// Stores one denied session-owned tool result and continues that session.
async fn session_deny(session_id: SessionId, tool_call_id: ToolCallId) -> Result<()> {
    operation::deny_cli_session_tool(session_id, tool_call_id, gateway_url(), base_url()).await
}

/// Cancels one persisted session.
fn session_stop(session_id: SessionId) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;
    let (session, _) = operation::cancel_session(&mut store, &session_id)?;
    output.session_status(&session);

    Ok(())
}

/// Persists the default model for future turns in one conversation.
fn set_model(conversation_id: ConversationId, model: ModelName) -> Result<()> {
    let mut store = Store::open()?;
    let output = TerminalOutput;

    operation::set_conversation_model(&mut store, &conversation_id, &model)?;
    output.set_model(&conversation_id, &model);

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
    let report = operation::start_gateway(gateway_url()).await?;
    output.component_report(&report);
    Ok(())
}

/// Stops the local Bifrost gateway process owned by the configured port.
async fn stop_gateway() -> Result<()> {
    let output = TerminalOutput;
    let report = operation::stop_gateway(gateway_url()).await?;
    output.component_report(&report);
    Ok(())
}

/// Centralizes the gateway health base URL.
fn gateway_url() -> GatewayUrl {
    GatewayUrl::new(config::gateway_url())
}

/// Centralizes the OpenAI-compatible API base URL.
fn base_url() -> BaseUrl {
    BaseUrl::new(
        std::env::var("WINDIE_BASE_URL")
            .unwrap_or_else(|_| format!("{}/v1", gateway_url().as_str())),
    )
}

/// Centralizes the local developer API bind address.
fn api_address() -> SocketAddr {
    config::api_address()
        .parse()
        .expect("WINDIE_API_ADDRESS must be a valid socket address")
}
