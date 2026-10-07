//! Unit tests of `apply.rs`: the flag parsers and the wording helpers.

use super::*;

#[test]
fn tags_split_at_the_first_equals_sign_and_names_are_upper_cased() {
    let t = parse_tag("comment=a=b").expect("parsed");
    assert_eq!((t.name.as_str(), t.value.as_str()), ("COMMENT", "a=b"));
    let t = parse_tag("BPM=").expect("parsed; the value is checked before the run");
    assert_eq!(t.value, "");
    assert!(parse_tag("BPM").is_err_and(|e| e.contains("NAME=VALUE")));
    assert!(parse_tag("=128").is_err_and(|e| e.contains("no name")));
}

#[test]
fn bits_and_gain_are_checked() {
    assert_eq!(parse_bits("16"), Ok(16));
    assert_eq!(parse_bits("24"), Ok(24));
    assert!(parse_bits("32").is_err() && parse_bits("8").is_err());
    assert_eq!(parse_gain("-3.2"), Ok(-3.2));
    assert!(parse_gain("nan").is_err() && parse_gain("inf").is_err());
    assert!(parse_gain("200").is_err() && parse_gain("loud").is_err());
}

#[test]
fn counts_agree_with_their_noun() {
    assert_eq!(plural(0_u64, "frame", "frames"), "0 frames");
    assert_eq!(plural(1_u32, "tag", "tags"), "1 tag");
    assert_eq!(plural(27_u64, "block", "blocks"), "27 blocks");
    assert!((ms(Duration::from_micros(1_234_567)) - 1234.567).abs() < 1e-9);
}
