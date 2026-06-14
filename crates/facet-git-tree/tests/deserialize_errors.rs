//! Integration tests for deserialization error paths.
//!
//! These exercise the `Error` variants that only arise on read, against missing,
//! mistyped, or externally-produced (foreign) trees — the robustness a git interop
//! library must have:
//!   Error::NotFound    — a referenced object is absent from the store
//!   Error::NotATree    — an object expected to be a tree is another kind
//!   Error::NonUtf8Name — a foreign tree has a non-UTF-8 entry name

use facet_git_tree::{EntryKind, EntryMode, Error, ObjectStore, TreeEntry, deserialize};
use gix_object::bstr::BString;
use gix_object::{Kind, Tree, Write};

mod common;
use common::Point;

/// Deserializing from a root id absent from the store yields `NotFound`.
#[test]
#[ignore = "deserialization not yet implemented"]
fn missing_root_object_is_not_found() {
    // An id produced in one store, queried in an empty one, is absent.
    let written = ObjectStore::default();
    let id = written.write_buf(Kind::Blob, b"hello").expect("write blob");

    let empty = ObjectStore::default();
    let result: Result<Point, _> = deserialize(&id, &empty);
    assert!(
        matches!(result, Err(Error::NotFound(_))),
        "absent root must be NotFound"
    );
}

/// Deserializing where the root id points to a blob yields `NotATree`.
#[test]
#[ignore = "deserialization not yet implemented"]
fn blob_root_is_not_a_tree() {
    let store = ObjectStore::default();
    let blob_id = store
        .write_buf(Kind::Blob, b"not a tree")
        .expect("write blob");

    let result: Result<Point, _> = deserialize(&blob_id, &store);
    assert!(
        matches!(result, Err(Error::NotATree(_))),
        "blob root must be NotATree"
    );
}

/// A foreign tree with a non-UTF-8 entry name is rejected as `NonUtf8Name`.
#[test]
#[ignore = "deserialization not yet implemented"]
fn non_utf8_entry_name_is_rejected() {
    let store = ObjectStore::default();
    let blob = store.write_buf(Kind::Blob, b"v").expect("write blob");

    // Only an externally-produced tree can contain a non-UTF-8 name, which is
    // exactly the foreign input read must tolerate (and reject cleanly).
    let tree = Tree {
        entries: vec![TreeEntry {
            mode: EntryMode::from(EntryKind::Blob),
            filename: BString::from(vec![0xff_u8, 0xfe]),
            oid: blob,
        }],
    };
    let tree_id = store.write(&tree).expect("write tree");

    let result: Result<Point, _> = deserialize(&tree_id, &store);
    assert!(
        matches!(result, Err(Error::NonUtf8Name(_))),
        "non-UTF-8 entry name must be NonUtf8Name"
    );
}
