//! Canonical, storage-independent conversation-tree rules.
//!
//! SQLite and PostgreSQL persist the same parent-linked message graph but use
//! different SQL and transaction mechanics. This module owns only the shared
//! graph rules: validating parent links, resolving a selected root-to-head
//! path, planning a splice delete, and finding descendants for truncation.
//! It deliberately contains no database, account, HTTP, session, or tool
//! execution behavior.

use std::collections::{BTreeMap, BTreeSet};

/// One persisted message link needed for canonical tree policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationTreeNode {
    /// Stable persisted message identifier.
    pub(crate) id: String,
    /// Parent identifier, or `None` for a root message.
    pub(crate) parent_message_id: Option<String>,
}

/// Validated canonical parent graph for one conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationTree {
    parents: BTreeMap<String, Option<String>>,
}

/// A semantic tree failure, independent of a database or transport.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConversationTreeError {
    /// A requested message does not occur in the current conversation tree.
    #[error("message does not exist in this conversation: {0}")]
    MessageNotFound(String),
    /// A proposed new parent does not occur in the current conversation tree.
    #[error("message parent does not belong to this conversation: {0}")]
    ParentNotFound(String),
    /// Persisted parent links cannot form a canonical rooted tree.
    #[error("conversation message tree is invalid: {0}")]
    InvalidTree(String),
    /// A delete plan must remove at least one existing message.
    #[error("splice delete must contain at least one message")]
    EmptyDelete,
    /// A splice parent cannot also be removed by the same plan.
    #[error("splice parent cannot be removed by the same delete")]
    DeletedSpliceParent,
}

/// A database-independent plan for removing messages while preserving branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpliceDeletePlan {
    /// All messages that must be deleted together.
    pub(crate) deleted_message_ids: BTreeSet<String>,
    /// The parent to which direct surviving children are reconnected.
    pub(crate) splice_parent_message_id: Option<String>,
    /// Direct children of deleted messages that survive the delete.
    pub(crate) promoted_child_ids: Vec<String>,
}

/// A database-independent plan for pruning descendants after a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TruncatePlan {
    /// Every descendant to delete. The checkpoint itself is never included.
    pub(crate) deleted_message_ids: BTreeSet<String>,
}

impl ConversationTree {
    /// Builds and validates one canonical tree from persisted message links.
    pub(crate) fn new(
        nodes: impl IntoIterator<Item = ConversationTreeNode>,
    ) -> Result<Self, ConversationTreeError> {
        let mut parents = BTreeMap::new();
        for node in nodes {
            if parents
                .insert(node.id.clone(), node.parent_message_id)
                .is_some()
            {
                return Err(ConversationTreeError::InvalidTree(format!(
                    "message identifier is duplicated: {}",
                    node.id
                )));
            }
        }

        for (message_id, parent_message_id) in &parents {
            if let Some(parent_message_id) = parent_message_id
                && !parents.contains_key(parent_message_id)
            {
                return Err(ConversationTreeError::InvalidTree(format!(
                    "message {message_id} references missing parent {parent_message_id}"
                )));
            }
        }

        for message_id in parents.keys() {
            let mut seen = BTreeSet::new();
            let mut cursor = Some(message_id.as_str());
            while let Some(current) = cursor {
                if !seen.insert(current) {
                    return Err(ConversationTreeError::InvalidTree(format!(
                        "parent cycle contains message {current}"
                    )));
                }
                cursor = parents
                    .get(current)
                    .ok_or_else(|| {
                        ConversationTreeError::InvalidTree(format!(
                            "message {current} disappeared while validating parents"
                        ))
                    })?
                    .as_deref();
            }
        }

        Ok(Self { parents })
    }

    /// Validates an existing message identifier.
    pub(crate) fn require_message(&self, message_id: &str) -> Result<(), ConversationTreeError> {
        if self.parents.contains_key(message_id) {
            Ok(())
        } else {
            Err(ConversationTreeError::MessageNotFound(
                message_id.to_string(),
            ))
        }
    }

    /// Validates an optional parent selected for a newly appended message.
    pub(crate) fn validate_append_parent(
        &self,
        parent_message_id: Option<&str>,
    ) -> Result<(), ConversationTreeError> {
        match parent_message_id {
            Some(parent_message_id) if !self.parents.contains_key(parent_message_id) => Err(
                ConversationTreeError::ParentNotFound(parent_message_id.to_string()),
            ),
            _ => Ok(()),
        }
    }

    /// Resolves the canonical root-to-selected-head path.
    pub(crate) fn selected_path(
        &self,
        head_message_id: &str,
    ) -> Result<Vec<String>, ConversationTreeError> {
        self.require_message(head_message_id)?;
        let mut path = Vec::new();
        let mut cursor = Some(head_message_id);
        while let Some(message_id) = cursor {
            path.push(message_id.to_string());
            cursor = self
                .parents
                .get(message_id)
                .ok_or_else(|| {
                    ConversationTreeError::InvalidTree(format!(
                        "message {message_id} disappeared while resolving path"
                    ))
                })?
                .as_deref();
        }
        path.reverse();
        Ok(path)
    }

    /// Plans removal of one ordinary message and promotion of its direct children.
    pub(crate) fn plan_remove_message(
        &self,
        message_id: &str,
    ) -> Result<SpliceDeletePlan, ConversationTreeError> {
        self.require_message(message_id)?;
        let deleted_message_ids = BTreeSet::from([message_id.to_string()]);
        self.plan_splice_delete(
            self.parents
                .get(message_id)
                .and_then(|parent_message_id| parent_message_id.as_deref()),
            deleted_message_ids,
        )
    }

    /// Plans promotion after a caller has selected a complete semantic delete set.
    ///
    /// Local tool-call groups may require deleting several linked messages at
    /// once. Their group-selection rule remains local-runtime specific, while
    /// this function remains the shared splice-link policy for either backend.
    pub(crate) fn plan_splice_delete(
        &self,
        splice_parent_message_id: Option<&str>,
        deleted_message_ids: BTreeSet<String>,
    ) -> Result<SpliceDeletePlan, ConversationTreeError> {
        if deleted_message_ids.is_empty() {
            return Err(ConversationTreeError::EmptyDelete);
        }
        for message_id in &deleted_message_ids {
            self.require_message(message_id)?;
        }
        if let Some(parent_message_id) = splice_parent_message_id {
            self.require_message(parent_message_id)?;
            if deleted_message_ids.contains(parent_message_id) {
                return Err(ConversationTreeError::DeletedSpliceParent);
            }
        }

        let mut promoted_child_ids = self
            .parents
            .iter()
            .filter(|(message_id, parent_message_id)| {
                parent_message_id
                    .as_ref()
                    .is_some_and(|parent| deleted_message_ids.contains(parent))
                    && !deleted_message_ids.contains(*message_id)
            })
            .map(|(message_id, _)| message_id.clone())
            .collect::<Vec<_>>();
        promoted_child_ids.sort();

        Ok(SpliceDeletePlan {
            deleted_message_ids,
            splice_parent_message_id: splice_parent_message_id.map(str::to_string),
            promoted_child_ids,
        })
    }

    /// Plans deletion of every descendant below one checkpoint message.
    pub(crate) fn plan_truncate_after(
        &self,
        checkpoint_message_id: &str,
    ) -> Result<TruncatePlan, ConversationTreeError> {
        self.require_message(checkpoint_message_id)?;
        let mut deleted_message_ids = BTreeSet::new();
        let mut frontier = vec![checkpoint_message_id.to_string()];
        while let Some(parent_message_id) = frontier.pop() {
            for child_message_id in self
                .parents
                .iter()
                .filter(|(_, parent)| parent.as_deref() == Some(parent_message_id.as_str()))
                .map(|(message_id, _)| message_id.clone())
            {
                if deleted_message_ids.insert(child_message_id.clone()) {
                    frontier.push(child_message_id);
                }
            }
        }
        Ok(TruncatePlan {
            deleted_message_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> ConversationTree {
        ConversationTree::new([
            ConversationTreeNode {
                id: "root".into(),
                parent_message_id: None,
            },
            ConversationTreeNode {
                id: "left".into(),
                parent_message_id: Some("root".into()),
            },
            ConversationTreeNode {
                id: "right".into(),
                parent_message_id: Some("root".into()),
            },
            ConversationTreeNode {
                id: "leaf".into(),
                parent_message_id: Some("left".into()),
            },
        ])
        .unwrap()
    }

    #[test]
    fn resolves_selected_path_from_root() {
        assert_eq!(
            tree().selected_path("leaf").unwrap(),
            ["root", "left", "leaf"]
        );
    }

    #[test]
    fn remove_plan_promotes_only_direct_surviving_children() {
        let plan = tree().plan_remove_message("left").unwrap();
        assert_eq!(plan.splice_parent_message_id.as_deref(), Some("root"));
        assert_eq!(plan.promoted_child_ids, ["leaf"]);
        assert_eq!(
            plan.deleted_message_ids,
            BTreeSet::from(["left".to_string()])
        );
    }

    #[test]
    fn truncate_plan_excludes_checkpoint_and_includes_all_descendants() {
        let plan = tree().plan_truncate_after("root").unwrap();
        assert_eq!(
            plan.deleted_message_ids,
            BTreeSet::from(["left".to_string(), "right".to_string(), "leaf".to_string()])
        );
    }

    #[test]
    fn rejects_missing_parent_and_cycles() {
        assert!(matches!(
            ConversationTree::new([ConversationTreeNode {
                id: "child".into(),
                parent_message_id: Some("missing".into()),
            }]),
            Err(ConversationTreeError::InvalidTree(_))
        ));
        assert!(matches!(
            ConversationTree::new([ConversationTreeNode {
                id: "self".into(),
                parent_message_id: Some("self".into()),
            }]),
            Err(ConversationTreeError::InvalidTree(_))
        ));
    }
}
