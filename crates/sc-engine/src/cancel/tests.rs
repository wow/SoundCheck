//! Unit tests of the private parts of `crates/sc-engine/src/cancel.rs`.
use super::*;

#[test]
fn clones_share_the_flag() {
    let token = CancelToken::new();
    let clone = token.clone();
    let flag = token.flag();
    assert!(!clone.is_cancelled());
    token.cancel();
    assert!(clone.is_cancelled());
    assert!(flag.load(Ordering::Relaxed));
}
