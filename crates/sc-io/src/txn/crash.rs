//! Crash injection for the crash matrix (feature `crash-test`, test builds only): the process
//! aborts at the point the environment variable [`CRASH_ENV`] names. The points are every
//! journaled state, right after its line (`planned`, `temp_written`, `verified`, `backed_up`,
//! `renamed`, `metadata_done`), and these places between them: `render` (the temp file was
//! just created and is still empty), `backup_copy` (the backup's temp copy is written, not yet
//! synced), `backup_named` (the backup's final name is journaled, the rename not done),
//! `placeholder` (an empty placeholder holds a no-replace target on a volume without exclusive
//! rename) and `rename` (the rename is done, its line not written). Without the feature this
//! compiles to nothing.

use super::journal::State;

/// The environment variable that names the point to crash at.
pub const CRASH_ENV: &str = "SC_TEST_CRASH_AFTER_STEP";

/// Aborts the process if [`CRASH_ENV`] names `point` (feature `crash-test` only).
#[cfg(feature = "crash-test")]
pub(crate) fn point(point: &str) {
    if std::env::var_os(CRASH_ENV).is_some_and(|v| v == point) {
        tracing::error!(stage = point, "crash injected");
        std::process::abort();
    }
}

/// Does nothing: crash injection needs the `crash-test` feature.
#[cfg(not(feature = "crash-test"))]
#[inline]
pub(crate) fn point(_point: &str) {}

/// [`point`] at the state `state`, right after its journal line.
pub(crate) fn after(state: State) {
    point(state.as_str());
}
