//! Table-driven tests for [`check_key`], the write-side tree-entry-name validator.
//!
//! Runnable today: `check_key` is independent of the not-yet-implemented
//! `serialize` that is expected to call it.

use std::collections::HashMap;

use facet_git_tree::{Error, check_key, serialize};
use rstest::rstest;

mod common;
use common::WithMap;

/// Names that are valid tree entry names and so are accepted.
#[rstest]
#[case("name")]
#[case("field0")]
#[case("0001")] // a zero-padded ordinal name is a perfectly ordinary key
#[case(".env")] // begins with '.' but is not a reserved sentinel
#[case("schema")] // no leading dot
#[case("")] // emptiness is not `check_key`'s concern
fn accepts_valid_keys(#[case] key: &str) {
    assert!(check_key(key).is_ok(), "{key:?} should be accepted");
}

/// Keys containing the path separator are rejected as [`Error::InvalidKey`].
#[rstest]
#[case("a/b")]
#[case("/")]
#[case("nested/key")]
#[case("trailing/")]
fn rejects_keys_with_slash(#[case] key: &str) {
    assert!(
        matches!(check_key(key), Err(Error::InvalidKey(_))),
        "{key:?} should be rejected as InvalidKey"
    );
}

/// Keys colliding with a reserved sentinel are rejected as [`Error::ReservedKey`].
#[rstest]
#[case(".schema")]
#[case(".variant")]
fn rejects_reserved_keys(#[case] key: &str) {
    assert!(
        matches!(check_key(key), Err(Error::ReservedKey(_))),
        "{key:?} should be rejected as ReservedKey"
    );
}

// --- integration: serialize must apply `check_key` to dynamic (map) keys ---

/// `serialize` rejects a map key containing the path separator, surfacing it as
/// [`Error::InvalidKey`] rather than emitting an invalid tree entry name.
#[test]
#[ignore = "serialization not yet implemented"]
fn serialize_rejects_map_key_with_slash() {
    let mut table = HashMap::new();
    table.insert("a/b".to_string(), "v".to_string());
    assert!(
        matches!(serialize(&WithMap { table }), Err(Error::InvalidKey(_))),
        "a map key with '/' must be rejected by serialize"
    );
}

/// `serialize` rejects a map key that collides with a reserved sentinel name,
/// surfacing it as [`Error::ReservedKey`] so user data can never shadow `.schema`.
#[test]
#[ignore = "serialization not yet implemented"]
fn serialize_rejects_reserved_map_key() {
    let mut table = HashMap::new();
    table.insert(".schema".to_string(), "v".to_string());
    assert!(
        matches!(serialize(&WithMap { table }), Err(Error::ReservedKey(_))),
        "a reserved map key must be rejected by serialize"
    );
}
