//! The words the JSON documents of `apply`, `undo`, `journal` and `recover` use for the
//! journal's kinds, states and outcomes: a stable camelCase vocabulary of schema 1, kept apart
//! from how the journal itself spells them.

use sc_io::txn::{Outcome, State, TxnKind};

/// `inPlace`, `toFolder` or `undo`.
#[must_use]
pub fn kind(kind: TxnKind) -> &'static str {
    match kind {
        TxnKind::InPlace => "inPlace",
        TxnKind::ToFolder => "toFolder",
        TxnKind::Undo => "undo",
    }
}

/// A transaction state: `planned`, `tempWritten`, `verified`, `backedUp`, `renamed`,
/// `metadataDone`, `done`, `failed`, `recovered` or `forgotten`.
#[must_use]
pub fn state(state: State) -> &'static str {
    match state {
        State::Planned => "planned",
        State::TempWritten => "tempWritten",
        State::Verified => "verified",
        State::BackedUp => "backedUp",
        State::Renamed => "renamed",
        State::MetadataDone => "metadataDone",
        State::Done => "done",
        State::Failed => "failed",
        State::Recovered => "recovered",
        State::Forgotten => "forgotten",
    }
}

/// How recovery ended a change: `completed` or `rolledBack`.
#[must_use]
pub fn outcome(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Completed => "completed",
        Outcome::RolledBack => "rolledBack",
    }
}
