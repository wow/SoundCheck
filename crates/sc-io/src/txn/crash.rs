//! Crash injection for the crash matrix (feature `crash-test`, test builds only): right after a
//! step is journaled, the process aborts when the environment variable
//! [`CRASH_ENV`] names that step (`planned`, `temp_written`, `verified`, `backed_up`,
//! `renamed`, `metadata_done`). Without the feature this compiles to nothing.

use super::journal::State;

/// The environment variable that names the step to crash after.
pub const CRASH_ENV: &str = "SC_TEST_CRASH_AFTER_STEP";

/// Aborts the process if [`CRASH_ENV`] names `state` (feature `crash-test` only).
#[cfg(feature = "crash-test")]
pub(crate) fn after(state: State) {
    if std::env::var_os(CRASH_ENV).is_some_and(|v| v == state.as_str()) {
        tracing::error!(stage = state.as_str(), "crash injected");
        std::process::abort();
    }
}

/// Does nothing: crash injection needs the `crash-test` feature.
#[cfg(not(feature = "crash-test"))]
#[inline]
pub(crate) fn after(_state: State) {}
