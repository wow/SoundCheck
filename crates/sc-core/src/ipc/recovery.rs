//! What the crash recovery run when the app starts found: file changes a crash interrupted,
//! finished or rolled back, and those left pending (a volume that is not mounted).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the start-up recovery stands (`recovery_status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum RecoveryStatus {
    /// Not finished yet.
    Running,
    /// Not run: there is no backup folder to look in.
    Skipped {
        /// Why.
        reason: String,
    },
    /// The journal could not be read; nothing was changed.
    Failed {
        /// What went wrong.
        message: String,
    },
    /// Done.
    #[serde(rename_all = "camelCase")]
    Finished {
        /// Changes finished or rolled back now.
        recovered: Vec<RecoveredChange>,
        /// Changes left for a later run.
        pending: Vec<PendingChange>,
    },
}

/// How recovery ended an interrupted change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryOutcome {
    /// The new version was already in place; its metadata and sidecar were finished.
    Completed,
    /// The new version was not in place; what the change had made was removed and the file is
    /// as it was.
    RolledBack,
}

/// A change recovery ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredChange {
    /// Journal id of the change.
    pub txn: String,
    /// The file it concerned.
    pub path: String,
    /// How it ended.
    pub outcome: RecoveryOutcome,
}

/// A change recovery left pending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    /// Journal id of the change.
    pub txn: String,
    /// The file it concerns.
    pub path: String,
    /// Why it was left (volume not mounted, busy, cleanup failed).
    pub reason: String,
}
