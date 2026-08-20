//! CLI adapters for parsed Windie commands.
//!
//! These modules are the terminal-facing boundary between typed CLI commands
//! and shared operation, persistence, process, and output components. They
//! contain command orchestration, but no command-line parsing.

use anyhow::Result;

use super::Command;

mod conversation;
mod message;
mod session;
mod system;
mod tool;

/// Dispatches one parsed command to its terminal adapter.
pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Dev(command) => crate::dev::run_dev(command).await,
        Command::Release(command) => crate::dev::run_release(command).await,
        Command::Marketplace(command) => crate::dev::run_marketplace(command).await,
        Command::Benchmark(command) => crate::dev::run_benchmark(command).await,
        Command::ApiStart => system::start_api_process(),
        Command::ApiStop => system::stop_api_process(),
        Command::ApiOutput => {
            system::output_component(crate::local::process::ManagedComponent::Api)
        }
        Command::ApiRun => system::run_api().await,
        Command::InspectorStart => system::start_inspector_process(),
        Command::InspectorStop => system::stop_inspector_process(),
        Command::InspectorOutput => {
            system::output_component(crate::local::process::ManagedComponent::Inspector)
        }
        Command::Onboard => system::onboard().await,
        Command::Help => system::print_help(),
        Command::Invalid => system::invalid_usage(),
        Command::Version => system::print_version(),
        Command::Env(command) => system::env_command(command),
        Command::Install { target } => system::install_target(&target),
        Command::Uninstall { yes, dry_run } => system::uninstall_windie(yes, dry_run).await,
        Command::GatewayStart => system::start_gateway().await,
        Command::GatewayStop => system::stop_gateway().await,
        Command::GatewayOutput => {
            system::output_component(crate::local::process::ManagedComponent::Gateway)
        }
        Command::TrayStart => system::start_tray_process(),
        Command::TrayStop => system::stop_tray_process(),
        Command::TrayOutput => {
            system::output_component(crate::local::process::ManagedComponent::Tray)
        }
        Command::TrayRun => system::run_tray(),
        Command::Status => system::status().await,
        Command::New => conversation::new_conversation().await,
        Command::List { json } => conversation::list_conversations(json),
        Command::Show(conversation_id) => conversation::show_conversation(conversation_id),
        Command::Tree(conversation_id) => conversation::show_tree(conversation_id),
        Command::Inspect {
            conversation_id,
            head_message_id,
            model,
        } => conversation::inspect_conversation(conversation_id, head_message_id, model),
        Command::Fork {
            conversation_id,
            message_id,
        } => conversation::fork_conversation(conversation_id, message_id),
        Command::SetModel {
            conversation_id,
            model,
        } => conversation::set_model(conversation_id, model),
        Command::RemoveConversation(conversation_id) => {
            conversation::remove_conversation(conversation_id)
        }
        Command::InsertMessage {
            conversation_id,
            head_message_id,
            role,
            parts,
        } => message::insert_message(conversation_id, head_message_id, role, &parts),
        Command::UpdateMessage {
            conversation_id,
            message_id,
            text,
        } => message::update_message(conversation_id, message_id, &text),
        Command::SetSystemPrompt {
            conversation_id,
            text,
        } => message::set_system_prompt(conversation_id, &text),
        Command::RemoveSystemPrompt(conversation_id) => {
            message::remove_system_prompt(conversation_id)
        }
        Command::RemoveMessage {
            conversation_id,
            message_id,
        } => message::remove_message(conversation_id, message_id),
        Command::Truncate {
            conversation_id,
            message_id,
        } => message::truncate_conversation(conversation_id, message_id),
        Command::Tools { provider_id } => tool::list_tools(provider_id),
        Command::AttachTool {
            conversation_id,
            provider_id,
            tool_name,
        } => tool::attach_tool(conversation_id, provider_id, tool_name),
        Command::DetachTool {
            conversation_id,
            schema_name,
        } => tool::detach_tool(conversation_id, schema_name),
        Command::InsertToolSchema {
            conversation_id,
            tool_schema,
        } => tool::insert_tool_schema(conversation_id, &tool_schema),
        Command::UpdateToolSchema {
            conversation_id,
            current_name,
            tool_schema,
        } => tool::update_tool_schema(conversation_id, current_name, &tool_schema),
        Command::RemoveToolSchema {
            conversation_id,
            name,
        } => tool::remove_tool_schema(conversation_id, name),
        Command::SessionStart {
            conversation_id,
            head_message_id,
            model,
        } => session::start(conversation_id, head_message_id, model).await,
        Command::SessionList { conversation_id } => session::list(conversation_id),
        Command::SessionStatus { session_id } => session::status(session_id),
        Command::SessionEvents { session_id } => session::events(session_id),
        Command::SessionApprovals { session_id } => session::approvals(session_id),
        Command::SessionApprove {
            session_id,
            tool_call_id,
        } => session::approve(session_id, tool_call_id).await,
        Command::SessionDeny {
            session_id,
            tool_call_id,
        } => session::deny(session_id, tool_call_id).await,
        Command::SessionStop { session_id } => session::stop(session_id),
        Command::Models => system::list_models().await,
    }
}
