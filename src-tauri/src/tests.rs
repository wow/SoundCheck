//! Unit tests of the private parts of `src-tauri/src/lib.rs`.
#[test]
fn version_matches_core() {
    assert_eq!(super::app_version(), sc_core::VERSION);
}
