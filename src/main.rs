//! Windie process entrypoint.
//!
//! The binary declares the runtime modules, parses one startup command, and
//! hands execution to the CLI adapter.

mod api;
mod cli;
mod context;
mod conversation;
mod doctor;
mod error;
mod gateway;
mod image_input;
mod llm;
mod mcp;
mod operation;
mod output;
mod paths;
mod perf;
mod policy;
mod provider_env;
mod run;
mod runtime;
mod store;
mod tool;
mod tool_provider;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::execute(cli::read()).await
}
