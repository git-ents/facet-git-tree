//! Byte sequences are bare binary blobs, not newline-terminated text or trees.

use facet::Facet;
use facet_git_tree::{EntryKind, ObjectStore, deserialize, serialize};
use gix_object::Kind;
use rstest::rstest;

mod common;
use common::{get_tree_entry_mode, roundtrip, typed};

#[derive(Debug, Facet, PartialEq)]
struct WithBytes {
    data: Vec<u8>,
}

#[derive(Debug, Facet, PartialEq)]
struct WithByteArray {
    hash: [u8; 4],
}

#[derive(Debug, Facet, PartialEq)]
struct TwoBuffers {
    a: Vec<u8>,
    b: Vec<u8>,
}

#[rstest]
#[case(vec![])]
#[case(vec![0])]
#[case(vec![0, 0, 0])]
#[case(vec![b'\n'])]
#[case(b"\x7fELF\x02\x01\x01".to_vec())]
#[case((0u8..=255).collect())]
fn vec_u8_roundtrip(#[case] bytes: Vec<u8>) {
    assert_eq!(
        roundtrip(WithBytes {
            data: bytes.clone()
        }),
        WithBytes { data: bytes }
    );
}

#[test]
fn byte_array_roundtrip() {
    let expected = WithByteArray {
        hash: [0xde, 0xad, 0xbe, 0xef],
    };
    assert_eq!(
        roundtrip(WithByteArray {
            hash: expected.hash
        }),
        expected
    );
}

#[rstest]
#[case(vec![])]
#[case(vec![0, 1, 2, b'\n', 0xff])]
#[case(vec![b'a', b'\n', b'\n'])]
fn vec_u8_is_one_blob_with_verbatim_contents(#[case] bytes: Vec<u8>) {
    let input = WithBytes {
        data: bytes.clone(),
    };
    let (root, store) = serialize(&input).expect("serialize");
    let (kind, oid) = get_tree_entry_mode(&store, &root, "data");
    assert_eq!(kind, EntryKind::Blob);
    assert_eq!(
        store.get_blob(&typed(&oid)).expect("read blob"),
        Some(bytes)
    );
    assert_eq!(
        deserialize::<WithBytes, _, _>(&root, &store).expect("decode"),
        input
    );
}

#[test]
fn fixed_array_is_one_binary_blob() {
    let input = WithByteArray {
        hash: [0, b'\n', 0xff, b'\n'],
    };
    let (root, store) = serialize(&input).expect("serialize");
    let (kind, oid) = get_tree_entry_mode(&store, &root, "hash");
    assert_eq!(kind, EntryKind::Blob);
    assert_eq!(
        store.get_blob(&typed(&oid)).expect("read blob"),
        Some(input.hash.to_vec())
    );
    assert_eq!(
        deserialize::<WithByteArray, _, _>(&root, &store).expect("decode"),
        input
    );
}

#[test]
fn byte_array_rejects_wrong_blob_lengths() {
    let store = ObjectStore::default();
    for bytes in [b"abc".as_slice(), b"abcde".as_slice()] {
        let oid = store.write_buf(Kind::Blob, bytes).expect("write blob");
        assert!(deserialize::<[u8; 4], _, _>(&oid, &store).is_err());
    }
}

#[test]
fn identical_buffers_dedup() {
    let (root, store) = serialize(&TwoBuffers {
        a: vec![1, 2, 3, 4],
        b: vec![1, 2, 3, 4],
    })
    .expect("serialize");
    let (_, a_oid) = get_tree_entry_mode(&store, &root, "a");
    let (_, b_oid) = get_tree_entry_mode(&store, &root, "b");
    assert_eq!(a_oid, b_oid);
}

#[test]
fn empty_bytes_share_marker_blob_but_not_marker_tree() {
    let (empty_bytes, store) = serialize(&Vec::<u8>::new()).expect("serialize bytes");
    assert_eq!(
        store.get_blob(&empty_bytes).expect("read bytes"),
        Some(vec![])
    );

    let (empty_list, list_store) = serialize(&Vec::<i32>::new()).expect("serialize list");
    let marker = list_store
        .get_tree(&empty_list)
        .expect("read marker")
        .expect("tree");
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].filename, "none");
    assert_eq!(marker[0].oid, empty_bytes);
    assert_ne!(empty_list, empty_bytes);
    assert_eq!(
        deserialize::<Vec<u8>, _, _>(&empty_bytes, &store).expect("decode"),
        Vec::<u8>::new()
    );
}

#[test]
fn byte_sequences_are_not_subject_to_the_tree_entry_cap() {
    let bytes = vec![0xabu8; 4097];
    let (root, store) = serialize(&bytes).expect("encode bytes");
    assert_eq!(
        store.get_blob(&root).expect("read bytes"),
        Some(bytes.clone())
    );
    assert_eq!(
        deserialize::<Vec<u8>, _, _>(&root, &store).expect("decode"),
        bytes
    );
}
