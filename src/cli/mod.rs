//! Startup command parsing for the Windie CLI.
//!
//! This module owns command-line arguments only. It maps raw argv text into
//! typed commands such as `new`, `ls`, `insert`, `update`, `run`, `gateway`,
//! and `bench`. It should not open the database, call Bifrost, or print output.

use std::path::PathBuf;

use crate::conversation::{ConversationId, MessageId, Role, ToolCallId};
use crate::llm::ModelName;
use crate::perf::{BenchmarkCategory, BenchmarkMode, BenchmarkOptions};
use crate::session::SessionId;
use crate::tool::{ProviderToolName, ToolProviderId, ToolSchema, ToolSchemaName};

mod bench;
mod command;
mod env;
mod message;
mod onboard;
mod parser;
mod session;
mod tool_schema;

#[cfg(test)]
mod tests;

pub use command::{Command, EnvCommand, InsertPart};
pub use onboard::TerminalOnboarding;
pub use parser::read;

use bench::*;
use env::*;
use message::*;
#[cfg(test)]
use parser::command_from_args;
use session::*;
use tool_schema::*;
