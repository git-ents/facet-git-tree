//! Table-driven tests for [`check_key`], the write-side tree-entry-name validator.

use facet_git_tree::{KeyError, check_key};
use rstest::rstest;

/// Names that are valid tree entry names and so are accepted. Leading-dot
/// keys are ordinary data — canonical git only refuses the exact names `.`
/// and `..`, not names that merely begin with a dot.
#[rstest]
#[case("name")]
#[case("field0")]
#[case("0001")] // a zero-padded ordinal name is a perfectly ordinary key
#[case(".env")]
#[case(".schema")]
#[case(".variant")]
fn accepts_valid_keys(#[case] key: &str) {
    assert!(check_key(key).is_ok(), "{key:?} should be accepted");
}

/// Names canonical git refuses outright when creating a tree — verified
/// against `git mktree`/`git fsck` — rejected here so this codec can never
/// emit a tree canonical git would call malformed.
#[rstest]
#[case("")] // empty filename: "error: empty filename in tree entry"
#[case(".")] // fsck's hasDot
#[case("..")] // fsck's hasDotdot
fn rejects_names_canonical_git_refuses(#[case] key: &str) {
    assert!(
        matches!(check_key(key), Err(KeyError { key: k }) if k == key),
        "{key:?} should be rejected as a name canonical git refuses"
    );
}

/// Keys containing the path separator are rejected as [`KeyError`], which
/// carries the offending key.
#[rstest]
#[case("a/b")]
#[case("/")]
#[case("nested/key")]
#[case("trailing/")]
fn rejects_keys_with_slash(#[case] key: &str) {
    assert!(
        matches!(check_key(key), Err(KeyError { key: k }) if k == key),
        "{key:?} should be rejected as KeyError carrying the offending key"
    );
}

/// The reserved presence-marker name (`"_"`, written for an empty collection
/// or `None`/`Null`) is rejected for the same reason `/` is: a real dynamic
/// entry named exactly that would otherwise be indistinguishable, on read,
/// from the marker.
#[test]
fn rejects_the_reserved_marker_key() {
    assert!(
        matches!(check_key("_"), Err(KeyError { key }) if key == "_"),
        "\"_\" must be rejected as the reserved presence-marker key"
    );
}

/// NUL-bearing keys are rejected here rather than at the object backend: NUL
/// terminates a tree entry's name in the on-disk format, so `gix` can only
/// report an opaque backend error (no key, no path, no hint), while
/// [`check_key`] still has the name in hand as user data and can report it as
/// a [`KeyError`].
#[rstest]
#[case("a\0b")]
#[case("\0")]
#[case("leading\0")]
fn rejects_keys_with_nul(#[case] key: &str) {
    assert!(
        matches!(check_key(key), Err(KeyError { key: k }) if k == key),
        "{key:?} should be rejected as KeyError carrying the offending key"
    );
}
