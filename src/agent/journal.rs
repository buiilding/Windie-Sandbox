//! Protected recovery journal for one device assignment at a time.
//!
//! This journal is deliberately separate from local conversations and sessions.
//! It records only delivery/execution/result recovery state, so reconnects can
//! retry the same assignment without treating an external action as safe to
//! repeat.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::device::{DeviceWorkAssignment, DeviceWorkResult};

use super::storage::Storage;

const RECORD: &str = "active-work.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkJournalState {
    Accepted,
    Executing,
    Completed,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkJournalEntry {
    pub assignment: DeviceWorkAssignment,
    pub state: WorkJournalState,
    pub result: Option<DeviceWorkResult>,
}

pub(crate) struct WorkJournal<'a> {
    storage: &'a Storage,
}

impl<'a> WorkJournal<'a> {
    pub(crate) fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub(crate) fn load(&self) -> Result<Option<WorkJournalEntry>> {
        self.storage.load_record(RECORD)
    }

    /// Records delivery before asking the server for execution authority.
    /// A different assignment cannot overwrite unfinished recovery state.
    pub(crate) fn accept(&self, assignment: DeviceWorkAssignment) -> Result<WorkJournalEntry> {
        if let Some(existing) = self.load()? {
            if existing.assignment.id == assignment.id {
                return Ok(existing);
            }
            ensure!(
                existing.state == WorkJournalState::Completed,
                "An earlier device assignment needs recovery before another can start"
            );
        }
        let entry = WorkJournalEntry {
            assignment,
            state: WorkJournalState::Accepted,
            result: None,
        };
        self.save(&entry)?;
        Ok(entry)
    }

    pub(crate) fn mark_executing(&self, entry: &mut WorkJournalEntry) -> Result<()> {
        ensure!(
            entry.state == WorkJournalState::Accepted,
            "Device work cannot enter execution from its current recovery state"
        );
        entry.state = WorkJournalState::Executing;
        self.save(entry)
    }

    pub(crate) fn complete(
        &self,
        entry: &mut WorkJournalEntry,
        result: DeviceWorkResult,
    ) -> Result<()> {
        ensure!(
            entry.state == WorkJournalState::Executing,
            "Device work cannot save a result before execution"
        );
        entry.state = WorkJournalState::Completed;
        entry.result = Some(result);
        self.save(entry)
    }

    /// A process found dead after external execution began. It must never
    /// automatically run the assignment again because side effects are unknown.
    pub(crate) fn mark_uncertain(&self, entry: &mut WorkJournalEntry) -> Result<()> {
        ensure!(
            entry.state == WorkJournalState::Executing,
            "Only executing work is uncertain"
        );
        entry.state = WorkJournalState::Uncertain;
        self.save(entry)
    }

    fn save(&self, entry: &WorkJournalEntry) -> Result<()> {
        self.storage.save_record(RECORD, entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{CapabilityRevision, DeviceWork, DeviceWorkId, LeaseId};

    fn assignment() -> DeviceWorkAssignment {
        DeviceWorkAssignment {
            id: DeviceWorkId::new(),
            lease_id: LeaseId::new(),
            capability_revision: CapabilityRevision::new(),
            execution_token: "token".into(),
            work: DeviceWork::ReadSkill {
                tool_call_id: "call".into(),
                plugin_id: "plugin".into(),
                skill_id: "skill".into(),
            },
            expires_at: 0,
        }
    }

    #[test]
    fn execution_is_not_repeated_after_an_uncertain_restart() {
        let root =
            std::env::temp_dir().join(format!("windie-agent-journal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let storage = Storage::at(root.join("agent")).unwrap();
        let journal = WorkJournal::new(&storage);
        let mut entry = journal.accept(assignment()).unwrap();
        journal.mark_executing(&mut entry).unwrap();
        journal.mark_uncertain(&mut entry).unwrap();
        assert_eq!(
            journal.load().unwrap().unwrap().state,
            WorkJournalState::Uncertain
        );
        assert!(journal.accept(assignment()).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
