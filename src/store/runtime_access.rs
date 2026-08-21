//! Durable ownership record for one local Windie runtime.
//!
//! The runtime remains entirely local, but its API is reachable from the
//! hosted Inspector. This table records the one hosted-account subject that
//! the local machine owner explicitly paired with this database.

use super::*;

/// The hosted account currently authorized to use this local runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAccess {
    /// Stable Supabase Auth user identifier (`sub` / user id).
    pub account_id: String,
    /// Unix timestamp in milliseconds when the local user approved pairing.
    pub linked_at: i64,
}

/// Result of atomically pairing a hosted account with the local runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAccessLink {
    /// This call created the first local pairing.
    Linked(RuntimeAccess),
    /// The calling account was already the paired owner.
    AlreadyLinked(RuntimeAccess),
    /// A different hosted account already owns the runtime.
    OwnedByAnotherAccount,
}

impl Store {
    /// Loads the one account approved to use this local Windie database.
    pub fn runtime_access(&self) -> Result<Option<RuntimeAccess>> {
        self.connection
            .query_row(
                "SELECT account_id, linked_at FROM runtime_access WHERE singleton = 1",
                [],
                |row| {
                    Ok(RuntimeAccess {
                        account_id: row.get(0)?,
                        linked_at: row.get(1)?,
                    })
                },
            )
            .optional()
            .context("failed to load local runtime access")
    }

    /// Records the first explicit hosted-account pairing without ever replacing
    /// an existing owner. SQLite's singleton primary key makes concurrent
    /// pairing attempts deterministic.
    pub fn link_runtime_access(&self, account_id: &str) -> Result<RuntimeAccessLink> {
        let linked_at = now_millis()?;
        let created = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO runtime_access (singleton, account_id, linked_at)
                 VALUES (1, ?1, ?2)",
                params![account_id, linked_at],
            )
            .context("failed to link local runtime access")?;

        let access = self
            .runtime_access()?
            .context("local runtime access was not persisted")?;

        if access.account_id != account_id {
            return Ok(RuntimeAccessLink::OwnedByAnotherAccount);
        }

        Ok(if created == 1 {
            RuntimeAccessLink::Linked(access)
        } else {
            RuntimeAccessLink::AlreadyLinked(access)
        })
    }

    /// Removes the caller's local pairing so a later explicit pairing can
    /// connect a different hosted account.
    pub fn unlink_runtime_access(&self, account_id: &str) -> Result<bool> {
        let removed = self
            .connection
            .execute(
                "DELETE FROM runtime_access WHERE singleton = 1 AND account_id = ?1",
                params![account_id],
            )
            .context("failed to unlink local runtime access")?;
        Ok(removed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_access_is_single_owner_and_can_be_unlinked() {
        let store = Store::open_memory().unwrap();

        assert!(store.runtime_access().unwrap().is_none());
        assert!(matches!(
            store.link_runtime_access("account-a").unwrap(),
            RuntimeAccessLink::Linked(_)
        ));
        assert!(matches!(
            store.link_runtime_access("account-a").unwrap(),
            RuntimeAccessLink::AlreadyLinked(_)
        ));
        assert_eq!(
            store.link_runtime_access("account-b").unwrap(),
            RuntimeAccessLink::OwnedByAnotherAccount
        );
        assert!(!store.unlink_runtime_access("account-b").unwrap());
        assert!(store.unlink_runtime_access("account-a").unwrap());
        assert!(store.runtime_access().unwrap().is_none());
    }
}
