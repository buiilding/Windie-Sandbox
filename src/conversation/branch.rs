//! Branch enumeration types.
//!
//! A branch is a computed view over the message tree: each leaf in the tree
//! defines one branch endpoint. Branches are not persisted rows; they are
//! derived from the messages and sessions tables at read time.

use serde::Serialize;

/// One branch endpoint in a conversation tree.
///
/// Each leaf message (a message with no children) is one branch tip. The branch
/// may have an active session running from its tip, or it may be dormant.
#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    /// The leaf message at the tip of this branch.
    pub leaf_message_id: String,
    /// The parent of the leaf (the node where this branch diverged from
    /// siblings, or None for root).
    pub parent_message_id: Option<String>,
    /// The message where the session on this branch started. When this differs
    /// from the leaf's parent, the session forked from an earlier node.
    pub session_start_head_id: Option<String>,
    /// The active session on this branch, if any.
    pub active_session: Option<RunningSessionInfo>,
}

/// Lightweight session status for a branch that has an active session.
#[derive(Debug, Clone, Serialize)]
pub struct RunningSessionInfo {
    pub session_id: String,
    pub status: String,
}