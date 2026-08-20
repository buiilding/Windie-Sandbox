//! Shared Windie runtime library.
//!
//! The public `windie` CLI uses these modules for both runtime and repository
//! workflows so development tooling cannot drift from the runtime it
//! supervises.

#![allow(dead_code, private_bounds, private_interfaces)]

pub mod api;
pub mod cli;
pub mod config;
pub mod conversation;
pub mod dev;
pub mod error;
pub mod input;
pub mod llm;
pub mod local;
pub mod managed_runtime;
pub mod mcp;
pub mod operation;
pub mod output;
pub mod perf;
pub mod plugin;
pub mod runtime;
pub mod session;
pub mod store;
pub mod tool;
