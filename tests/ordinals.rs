//! Positional entries use fixed-width, lowercase hexadecimal ordinals.

use facet_git_tree::{EntryKind, ObjectStore, Oid, Sha1, deserialize, serialize};
use gix_object::Kind;
use proptest::prelude::*;

mod common;
use common::{WithArray, WithVec, get_tree_entry_mode, tree_entries, typed};

#[test]
fn vec_elements_use_three_digit_hexadecimal_names() {
    let input = WithVec {
        items: (0..17).collect(),
    };
    let (root, store) = serialize(&input).expect("serialize");
    let (_, items) = get_tree_entry_mode(&store, &root, "items");
    let entries = tree_entries(&store, &items);
    let names: Vec<_> = entries
        .iter()
        .map(|entry| entry.filename.to_string())
        .collect();
    assert_eq!(
        names,
        [
            "000", "001", "002", "003", "004", "005", "006", "007", "008", "009", "00a", "00b",
            "00c", "00d", "00e", "00f", "010"
        ]
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry.mode.kind() == EntryKind::Blob)
    );
    assert_eq!(
        deserialize::<WithVec, _, _>(&root, &store).expect("deserialize"),
        input
    );
}

#[test]
fn non_byte_array_elements_use_three_digit_hexadecimal_names() {
    let input = WithArray {
        values: [10, 20, 30, 40],
    };
    let (root, store) = serialize(&input).expect("serialize");
    let (_, array) = get_tree_entry_mode(&store, &root, "values");
    let names: Vec<_> = tree_entries(&store, &array)
        .iter()
        .map(|entry| entry.filename.to_string())
        .collect();
    assert_eq!(names, ["000", "001", "002", "003"]);
    assert_eq!(
        deserialize::<WithArray, _, _>(&root, &store).expect("deserialize"),
        input
    );
}

#[test]
fn entry_cap_is_4096_with_fff_as_last_ordinal() {
    let items: Vec<i64> = (0..4096).collect();
    let input = WithVec { items };
    let (root, store) = serialize(&input).expect("4096 entries fit");
    let (_, sequence) = get_tree_entry_mode(&store, &root, "items");
    let entries = tree_entries(&store, &sequence);
    assert_eq!(entries.len(), 4096);
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry.filename.to_string(), format!("{index:03x}"));
    }
    let decoded: WithVec = deserialize(&root, &store).expect("decode 4096 entries");
    assert_eq!(decoded, input);
    assert_eq!(serialize(&decoded).expect("re-encode").0, root);
    assert!(
        serialize(&vec![0i64; 4097]).is_err(),
        "4097 entries must be rejected"
    );
}

fn foreign_tree(store: &ObjectStore, names: &[&[u8]]) -> Oid<Sha1> {
    let value = store.write_buf(Kind::Blob, b"1\n").expect("write value");
    let mut bytes = Vec::new();
    for name in names {
        bytes.extend_from_slice(b"100644 ");
        bytes.extend_from_slice(name);
        bytes.push(0);
        bytes.extend_from_slice(value.as_bytes());
    }
    store.write_buf(Kind::Tree, &bytes).expect("write raw tree")
}

#[test]
fn rejects_foreign_width_case_gaps_duplicates_and_order() {
    let store = ObjectStore::default();
    for names in [
        vec![b"0".as_slice()],
        vec![b"00".as_slice()],
        vec![b"0000".as_slice()],
        vec![b"00g".as_slice()],
        vec![b"001".as_slice()],
        vec![b"000".as_slice(), b"002"],
        vec![b"000".as_slice(), b"000"],
        vec![b"001".as_slice(), b"000"],
    ] {
        let root = foreign_tree(&store, &names);
        assert!(
            deserialize::<Vec<i64>, _, _>(&root, &store).is_err(),
            "accepted {names:?}"
        );
    }

    let mut names: Vec<Vec<u8>> = (0..=10)
        .map(|index| format!("{index:03x}").into_bytes())
        .collect();
    names[10] = b"00A".to_vec();
    let root = foreign_tree(&store, &names.iter().map(Vec::as_slice).collect::<Vec<_>>());
    assert!(
        deserialize::<Vec<i64>, _, _>(&root, &store).is_err(),
        "uppercase hex must be rejected"
    );
}

#[test]
fn array_length_must_match_ordinal_count() {
    let store = ObjectStore::default();
    for names in [
        vec![b"000".as_slice()],
        vec![b"000".as_slice(), b"001", b"002"],
    ] {
        let root = foreign_tree(&store, &names);
        assert!(deserialize::<[i64; 2], _, _>(&root, &store).is_err());
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    #[test]
    fn sequence_ordinals_are_contiguous_lowercase_hex(items in proptest::collection::vec(-100i64..100, 1..40)) {
        let (root, store) = serialize(&WithVec { items: items.clone() }).expect("serialize");
        let (_, sequence) = get_tree_entry_mode(&store, &root, "items");
        let names: Vec<_> = tree_entries(&store, &sequence)
            .into_iter()
            .map(|entry| entry.filename.to_string())
            .collect();
        let expected: Vec<_> = (0..items.len()).map(|index| format!("{index:03x}")).collect();
        prop_assert_eq!(names, expected);
    }

    #[test]
    fn sequence_order_survives_serialization(items in proptest::collection::vec(-100i64..100, 0..40)) {
        let (root, store) = serialize(&WithVec { items: items.clone() }).expect("serialize");
        let recovered: WithVec = deserialize(&root, &store).expect("deserialize");
        prop_assert_eq!(recovered, WithVec { items });
    }
}

#[test]
fn empty_sequence_uses_none_tree_and_empty_blob() {
    let input = WithVec { items: vec![] };
    let (root, store) = serialize(&input).expect("serialize");
    let (_, sequence) = get_tree_entry_mode(&store, &root, "items");
    let entries = tree_entries(&store, &sequence);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "none");
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store
            .get_blob(&typed(&entries[0].oid))
            .expect("read marker"),
        Some(vec![])
    );
    assert_eq!(
        deserialize::<WithVec, _, _>(&root, &store).expect("deserialize"),
        input
    );
}
