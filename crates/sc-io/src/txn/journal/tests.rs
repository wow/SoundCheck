//! Unit tests of `journal.rs`: state names and order, folding lines into entries, cut lines,
//! undo marking, and the per-transaction lock.

use super::*;

fn planned(txn: &str, kind: TxnKind, path: &str) -> Line {
    let mut l = Line::new(txn, State::Planned);
    l.kind = Some(kind);
    l.path = Some(PathBuf::from(path));
    l.source = Some(PathBuf::from(path));
    l.temp = Some(PathBuf::from(format!("{path}.tmp")));
    l
}

#[test]
fn state_names_are_the_serialised_names_and_follow_the_steps() {
    let all = [
        State::Planned,
        State::TempWritten,
        State::Verified,
        State::BackedUp,
        State::Renamed,
        State::MetadataDone,
        State::Done,
        State::Failed,
        State::Recovered,
    ];
    for s in all {
        let json = serde_json::to_string(&s).expect("serialises");
        assert_eq!(json, format!("\"{}\"", s.as_str()));
    }
    assert!(all.windows(2).all(|w| w[0] < w[1]));
    assert!(State::Done.is_final() && State::Failed.is_final() && State::Recovered.is_final());
    assert!(!State::MetadataDone.is_final());
    assert_eq!(TxnKind::InPlace.pre_rename(), State::BackedUp);
    assert_eq!(TxnKind::ToFolder.pre_rename(), State::Verified);
    assert_eq!(TxnKind::Undo.pre_rename(), State::Verified);
}

#[test]
fn lines_fold_into_one_entry_per_transaction() {
    let mut verified = Line::new("a", State::Verified);
    verified.output_blake3 = Some("out".into());
    let mut backed = Line::new("a", State::BackedUp);
    backed.backup = Some(PathBuf::from("/b/x.wav"));
    backed.original_blake3 = Some("orig".into());
    let mut failed = Line::new("b", State::Failed);
    failed.error = Some("cancelled".into());
    let lines = vec![
        planned("a", TxnKind::InPlace, "/m/x.wav"),
        Line::new("a", State::TempWritten),
        planned("b", TxnKind::ToFolder, "/o/y.wav"),
        verified,
        failed,
        backed,
        // A line of a transaction whose planned line is missing.
        Line::new("ghost", State::Done),
    ];
    let entries = fold(lines);
    assert_eq!(entries.len(), 2);
    let a = &entries[0];
    assert_eq!(
        (a.txn.as_str(), a.state, a.reached),
        ("a", State::BackedUp, State::BackedUp)
    );
    assert_eq!(a.output_blake3.as_deref(), Some("out"));
    assert_eq!(a.original_blake3.as_deref(), Some("orig"));
    assert_eq!(a.backup.as_deref(), Some(Path::new("/b/x.wav")));
    assert!(!a.completed());
    let b = &entries[1];
    assert_eq!((b.state, b.reached), (State::Failed, State::Planned));
    assert_eq!(b.error.as_deref(), Some("cancelled"));
}

#[test]
fn a_completed_undo_marks_what_it_undid() {
    let mut undo = planned("u", TxnKind::Undo, "/m/x.wav");
    undo.undoes = Some("a".into());
    let mut recovered = Line::new("u", State::Recovered);
    recovered.outcome = Some(Outcome::Completed);
    let entries = fold(vec![
        planned("a", TxnKind::InPlace, "/m/x.wav"),
        Line::new("a", State::Done),
        undo,
        recovered,
    ]);
    assert!(entries[0].completed() && entries[0].undone);
    assert!(entries[1].completed());

    let mut undo = planned("u", TxnKind::Undo, "/m/x.wav");
    undo.undoes = Some("a".into());
    let entries = fold(vec![
        planned("a", TxnKind::InPlace, "/m/x.wav"),
        Line::new("a", State::Done),
        undo,
        Line::new("u", State::Failed),
    ]);
    assert!(!entries[0].undone, "a failed undo undoes nothing");
}

#[test]
fn the_journal_appends_reads_and_skips_cut_lines() {
    let dir = tempfile::tempdir().expect("temp dir");
    let journal = Journal::open(dir.path()).expect("opened");
    assert!(
        journal.entries().expect("read").is_empty(),
        "no journal yet"
    );
    journal
        .append(&planned("a", TxnKind::InPlace, "/m/x.wav"))
        .expect("appended");
    let path = dir.path().join(JOURNAL_FILE);
    let mut text = std::fs::read_to_string(&path).expect("journal");
    assert!(text.ends_with('\n') && text.lines().count() == 1);
    // A crash in the middle of a write leaves a cut line.
    text.push_str("{\"txn\":\"a\",\"state\":\"temp_wr");
    std::fs::write(&path, &text).expect("cut");
    let entries = journal.entries().expect("read");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].state, State::Planned);
    assert!(journal.entry("a").expect("read").is_some());
    assert!(journal.entry("b").expect("read").is_none());
}

#[test]
fn a_held_lock_cannot_be_taken_and_is_removed_when_dropped() {
    let dir = tempfile::tempdir().expect("temp dir");
    let journal = Journal::open(dir.path()).expect("opened");
    let held = TxnLock::try_acquire(&journal, "t")
        .expect("io")
        .expect("free");
    assert!(TxnLock::try_acquire(&journal, "t").expect("io").is_none());
    let path = journal.lock_path("t");
    assert!(path.exists());
    drop(held);
    assert!(!path.exists());
    let again = TxnLock::try_acquire(&journal, "t").expect("io");
    assert!(again.is_some());
}

#[test]
fn a_torn_last_line_does_not_swallow_the_next_record() {
    let dir = tempfile::tempdir().expect("temp dir");
    let journal = Journal::open(dir.path()).expect("opened");
    journal
        .append(&planned("a", TxnKind::InPlace, "/m/x.wav"))
        .expect("appended");
    let path = dir.path().join(JOURNAL_FILE);
    let mut text = std::fs::read_to_string(&path).expect("journal");
    text.push_str("{\"txn\":\"a\",\"state\":\"temp_wr");
    std::fs::write(&path, &text).expect("torn");
    journal
        .append(&Line::new("a", State::TempWritten))
        .expect("appended after the torn line");
    journal
        .append(&planned("b", TxnKind::ToFolder, "/o/y.wav"))
        .expect("appended");
    let entries = journal.entries().expect("read");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].state, State::TempWritten);
    assert_eq!(entries[1].txn, "b");
    let text = std::fs::read_to_string(&path).expect("journal");
    assert!(text.lines().all(|l| l.starts_with('{')), "{text}");
}
