//! Unit tests of the private parts of `crates/sc-cli/src/labels.rs`.
use super::*;

#[test]
fn cells_with_commas_or_quotes_are_quoted() {
    assert_eq!(csv("Artist - Song.flac"), "Artist - Song.flac");
    assert_eq!(csv("Artist, The.flac"), "\"Artist, The.flac\"");
    assert_eq!(csv("12\" mix.flac"), "\"12\"\" mix.flac\"");
    assert_eq!(csv("#1 hit.flac"), "\"#1 hit.flac\"");
}
