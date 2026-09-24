//! The public surface compiles and the version constant is a semver string.

#[test]
fn version_is_semver() {
    let parts: Vec<&str> = sc_core::VERSION.split('.').collect();
    assert_eq!(parts.len(), 3, "{}", sc_core::VERSION);
    assert!(parts[0].parse::<u32>().is_ok());
}
