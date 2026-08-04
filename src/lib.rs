//! Shared Windie runtime library.
//!
//! The installed `windie` CLI and the repository-only `windie-dev` binary use
//! these modules so development tooling cannot drift from the runtime it
//! supervises.

#![allow(dead_code, private_bounds, private_interfaces)]

pub mod api;
pub mod cli;
pub mod config;
pub mod context;
pub mod conversation;
pub mod error;
pub mod gateway;
pub mod input;
pub mod llm;
pub mod local;
pub mod mcp;
pub mod operation;
pub mod output;
pub mod perf;
pub mod process;
pub mod runtime;
pub mod session;
pub mod store;
pub mod tool;
pub mod tool_provider;
pub mod tray;
pub mod wakeup;
