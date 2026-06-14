//! Tests for the in-memory [`ObjectStore`] — the `gix` `Write`/`Find` backend
//! and its owned accessors.
//!
//! Unlike the serialization suites, these exercise code that is runnable today,
//! independent of the not-yet-implemented `serialize`.

use facet_git_tree::{EntryKind, EntryMode, ObjectStore, TreeEntry};
use gix_object::{Find, Kind, Tree, Write};
use proptest::prelude::*;

mod common;
use common::HELLO_BLOB_OID;

/// `write_buf` (hence `write_stream`) stores a blob that `get_blob` returns verbatim.
#[test]
fn write_then_get_blob_roundtrips() {
    let store = ObjectStore::default();
    let id = store.write_buf(Kind::Blob, b"hello").expect("write blob");
    assert_eq!(store.get_blob(&id).expect("blob present"), b"hello");
}

/// The ID a write produces is exactly the SHA-1 git computes for the same blob
/// (`blob 5\0hello`), confirming Git v2 object-format compatibility.
#[test]
fn blob_oid_matches_git() {
    let store = ObjectStore::default();
    let id = store.write_buf(Kind::Blob, b"hello").expect("write blob");
    assert_eq!(id.as_bytes(), HELLO_BLOB_OID.as_slice());
}

/// A tree's OID is exactly the SHA-1 git computes for the same canonical tree
/// encoding, confirming Git v2 tree-format compatibility — the property the whole
/// crate rests on. Blob hashing is trivial; tree hashing (entry modes, NUL-
/// terminated names, raw OID bytes, name sorting) is where compatibility is hard.
///
/// Ground truth: `printf 'hello' | git hash-object --stdin -w` then
/// `printf '100644 blob <hello>\ta\n100644 blob <hello>\tb\n' | git mktree`.
#[test]
fn tree_oid_matches_git() {
    let store = ObjectStore::default();
    let hello = store.write_buf(Kind::Blob, b"hello").expect("write blob");

    let blob = EntryMode::from(EntryKind::Blob);
    let tree = Tree {
        entries: vec![
            TreeEntry {
                mode: blob,
                filename: "a".into(),
                oid: hello,
            },
            TreeEntry {
                mode: blob,
                filename: "b".into(),
                oid: hello,
            },
        ],
    };
    let tree_id = store.write(&tree).expect("write tree");

    const EXPECTED_TREE_OID: [u8; 20] = [
        0xbc, 0x78, 0x7e, 0x44, 0x6f, 0x83, 0xed, 0x26, 0x0a, 0x7e, 0x22, 0xc6, 0x55, 0x83, 0xe1,
        0xbf, 0x14, 0x23, 0x81, 0x3e,
    ];
    assert_eq!(
        tree_id.as_bytes(),
        EXPECTED_TREE_OID.as_slice(),
        "tree OID must match git's canonical tree encoding"
    );
}

/// Accessors return `None` for an object absent from the store.
#[test]
fn get_absent_object_is_none() {
    let written = ObjectStore::default();
    let id = written.write_buf(Kind::Blob, b"hello").expect("write blob");

    let empty = ObjectStore::default();
    assert!(empty.get(&id).is_none());
    assert!(empty.get_blob(&id).is_none());
    assert!(empty.get_tree(&id).is_none());
}

/// The typed accessors reject objects of the wrong kind.
#[test]
fn typed_accessors_reject_wrong_kind() {
    let store = ObjectStore::default();
    let blob_id = store.write_buf(Kind::Blob, b"data").expect("write blob");

    let tree = Tree {
        entries: vec![TreeEntry {
            mode: EntryMode::from(EntryKind::Blob),
            filename: "x".into(),
            oid: blob_id,
        }],
    };
    let tree_id = store.write(&tree).expect("write tree");

    // A blob is not a tree, and vice versa.
    assert!(store.get_blob(&blob_id).is_some());
    assert!(store.get_tree(&blob_id).is_none());
    assert!(store.get_tree(&tree_id).is_some());
    assert!(store.get_blob(&tree_id).is_none());
}

/// The `Find` impl returns the stored bytes and the correct object kind.
#[test]
fn find_returns_stored_bytes_and_kind() {
    let store = ObjectStore::default();
    let id = store.write_buf(Kind::Blob, b"payload").expect("write blob");

    let mut buf = Vec::new();
    let data = store
        .try_find(&id, &mut buf)
        .expect("find ok")
        .expect("object present");
    assert_eq!(data.kind, Kind::Blob);
    assert_eq!(data.data, b"payload");
}

/// `write_stream` treats the caller-supplied `size` as an untrusted hint: a
/// wildly over-large value neither corrupts the stored object nor changes its
/// ID — the prealloc cap keeps the allocation bounded while `read_to_end`
/// governs the actual length.
#[test]
fn write_stream_tolerates_a_bogus_size_hint() {
    let store = ObjectStore::default();
    let data = b"the actual bytes";

    let lied = store
        .write_stream(Kind::Blob, u64::MAX, &mut &data[..])
        .expect("write with bogus size");
    assert_eq!(store.get_blob(&lied).expect("present"), data);

    let honest = store.write_buf(Kind::Blob, data).expect("honest write");
    assert_eq!(lied, honest, "the size hint must not affect the object ID");
}

proptest! {
    /// Any byte string round-trips through `write_buf` + `get_blob`, and the
    /// resulting ID is deterministic across independent stores — the defining
    /// property of a content-addressed store.
    #[test]
    fn arbitrary_blob_roundtrips_deterministically(data: Vec<u8>) {
        let s1 = ObjectStore::default();
        let id1 = s1.write_buf(Kind::Blob, &data).expect("write into s1");
        prop_assert_eq!(s1.get_blob(&id1).expect("present in s1"), data.clone());

        let s2 = ObjectStore::default();
        let id2 = s2.write_buf(Kind::Blob, &data).expect("write into s2");
        prop_assert_eq!(id1, id2);
    }
}
