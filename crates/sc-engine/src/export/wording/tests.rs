use sc_core::export::ExportSkip;

use super::{skip_note, skip_why};

const WHAT_TO_DO: &str = "rename one, or choose another folder";

#[test]
fn a_copy_in_the_folder_says_it_may_be_this_file_before_it_moved() {
    let why = skip_why(ExportSkip::CopyOfOtherTrack);
    assert!(why.contains("made from another file"), "{why}");
    assert!(why.contains("before it was moved and changed"), "{why}");
}

#[test]
fn a_name_taken_in_the_batch_does_not_say_a_copy_exists() {
    let why = skip_why(ExportSkip::SameNameInBatch);
    assert!(why.contains("earlier in this export"), "{why}");
    assert!(!why.contains("already"), "nothing need be there yet: {why}");
    assert!(!why.contains("copy"), "nothing need be there yet: {why}");
}

#[test]
fn the_app_says_what_to_do_about_a_name_and_the_command_line_keeps_its_own() {
    for reason in [ExportSkip::CopyOfOtherTrack, ExportSkip::SameNameInBatch] {
        let note = skip_note(reason);
        assert!(note.starts_with(&skip_why(reason)), "{note}");
        assert!(note.ends_with(WHAT_TO_DO), "{note}");
        assert!(!skip_why(reason).contains(WHAT_TO_DO));
    }
}

#[test]
fn other_skips_keep_their_note() {
    let reason = ExportSkip::Silent;
    assert_eq!(skip_note(reason), skip_why(reason));
}

#[test]
fn the_words_read_as_the_app_shows_them() {
    for reason in [ExportSkip::CopyOfOtherTrack, ExportSkip::SameNameInBatch] {
        let note = skip_note(reason);
        // The app makes a sentence of it: starting small, ending with no stop of its own.
        assert!(note.starts_with(char::is_lowercase), "{note}");
        assert!(!note.ends_with('.'), "{note}");
        let lower = note.to_lowercase();
        assert!(!lower.contains("sorry") && !lower.contains("unfortunately"));
    }
}
