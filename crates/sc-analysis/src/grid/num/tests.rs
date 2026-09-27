//! Unit tests of the private parts of `crates/sc-analysis/src/grid/num.rs`.
use super::*;

#[test]
fn signed_shifts_wrap_around_the_bar() {
    assert_eq!(signed_shift(1, 0, 4), 1);
    assert_eq!(signed_shift(2, 0, 4), 2);
    assert_eq!(signed_shift(3, 0, 4), -1);
    assert_eq!(signed_shift(0, 3, 4), 1);
    assert_eq!(signed_shift(4, 0, 9), 4);
    assert_eq!(signed_shift(5, 0, 9), -4);
}
