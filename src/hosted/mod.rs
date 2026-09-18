//! Authenticated, PostgreSQL-backed Windie hosted-server boundary.
//!
//! This module deliberately does not reuse the local SQLite `Store` or the
//! localhost API. It preserves Windie's conversation-tree model while adding
//! account ownership, durable mutation idempotency, and replayable account
//! change events for independently connected browsers.

mod account;
mod api;
mod auth;
mod config;
mod conversation;
mod events;
mod runtime;
mod store;

pub use api::serve;
pub use config::HostedConfig;
pub use store::HostedStore;

// These are crate-visible hosted persistence contracts. The hosted
// conversation adapter keeps HTTP handlers thin without duplicating shared
// conversation-tree policy.
pub(crate) use account::HostedAccount;
pub(crate) use runtime::HostedRuntime;
pub(crate) use store::{
    HostedChangeEvent, HostedConversation, HostedConversationSummary, HostedMessagePartInput,
    HostedStoreError, MutationResponse,
};
