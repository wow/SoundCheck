//! The gate between the crash recovery run at start and the first write of an export: until
//! recovery has finished or rolled back what a crash interrupted, a file it is about to repair
//! must be neither analysed nor written, because a plan made from the wrong version of it
//! would apply its gain twice. The app shell starts recovery on its own thread and sets the
//! gate when it ends; `sc-cli` recovers first and opens the gate at once.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use sc_core::ipc::RecoveryStatus;
use sc_core::{Error, Result};

use crate::CancelToken;

/// How often a waiting export looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// Where the start-up recovery stands, shared by the thread running it and the jobs that write.
#[derive(Debug)]
pub struct RecoveryGate {
    status: Mutex<RecoveryStatus>,
    changed: Condvar,
}

impl RecoveryGate {
    /// A gate whose recovery stands at `status`; closed while it is
    /// [`RecoveryStatus::Running`].
    #[must_use]
    pub fn new(status: RecoveryStatus) -> Self {
        Self {
            status: Mutex::new(status),
            changed: Condvar::new(),
        }
    }

    /// A closed gate: recovery is about to run.
    #[must_use]
    pub fn running() -> Self {
        Self::new(RecoveryStatus::Running)
    }

    /// Records what recovery found (anything but `Running` opens the gate) and wakes every
    /// waiting job.
    pub fn set(&self, status: RecoveryStatus) {
        *self.lock() = status;
        self.changed.notify_all();
    }

    /// What recovery found so far.
    #[must_use]
    pub fn status(&self) -> RecoveryStatus {
        self.lock().clone()
    }

    /// Whether recovery is still running (the gate is closed).
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(*self.lock(), RecoveryStatus::Running)
    }

    /// Blocks while recovery runs.
    ///
    /// # Errors
    /// [`Error::Cancelled`] once `cancel` is set while waiting.
    pub fn wait(&self, cancel: &CancelToken) -> Result<()> {
        let mut status = self.lock();
        while matches!(*status, RecoveryStatus::Running) {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            status = self
                .changed
                .wait_timeout(status, CANCEL_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RecoveryStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
