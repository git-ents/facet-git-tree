//! Integration tests for built-in collection type serialization.
//!
//! String-keyed maps and non-byte sequences are Git trees; non-string-keyed
//! maps are canonical CBOR blobs. Empty native collections use the `none` marker.
//!
//! Ordinal entry naming for sequence collections is covered in `ordinals.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use facet::Facet;
use facet_git_tree::{
    DeserializeError, EntryKind, EntryMode, Node, ObjectStore, SerializeError, TreeEntry,
    deserialize, schema_of, serialize,
};
use gix_object::{Kind, Tree, Write};
use proptest::prelude::*;
mod common;
use common::{WithArray, WithMap, WithVec, get_tree_entry_mode, tree_entries};

#[derive(Facet, PartialEq, Debug)]
struct WithIntMap {
    table: HashMap<u32, String>,
}

#[derive(Facet, PartialEq, Debug)]
struct WithArcStrKeyMap {
    table: HashMap<Arc<str>, u32>,
}

#[derive(Facet, PartialEq, Eq, Hash, Ord, PartialOrd, Debug, Clone)]
struct Coord {
    x: i32,
    y: i32,
}

#[derive(Facet, PartialEq, Debug)]
struct WithCompositeKeyMap {
    table: HashMap<Coord, String>,
}

/// A float-backed composite key, for the duplicate-key suites: two `NaN`s
/// compare unequal (so a `HashMap` keeps both) while both encode to the
/// same canonical CBOR NaN key.
#[derive(Facet, Clone, Debug, PartialEq)]
struct FloatKey {
    x: f64,
}

impl Eq for FloatKey {}

impl std::hash::Hash for FloatKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Any NaN hashes alike (all encode to the same CBOR key) while comparing
        // unequal, so a HashMap keeps two of them — the broken-key-equality
        // case the duplicate-key check exists to catch.
        if self.x.is_nan() {
            0u64.hash(state);
        } else {
            self.x.to_bits().hash(state);
        }
    }
}

#[derive(Facet)]
struct WithFloatKeyMap {
    table: HashMap<FloatKey, String>,
}

// --- Vec ---

/// A Vec field is encoded as a tree (not a blob) holding only its elements.
#[test]
fn vec_field_is_tree() {
    let (root_id, store) = serialize(&WithVec {
        items: vec![1, 2, 3],
    })
    .expect("serialize should succeed");

    let (mode, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    assert_eq!(mode, EntryKind::Tree, "Vec field must be a tree");
    assert_eq!(
        tree_entries(&store, &items_id).len(),
        3,
        "Vec with 3 elements should have 3 entries"
    );
}

/// An empty Vec serializes to the presence-marker tree, not a literal empty
/// tree: a literal empty tree contributes nothing to `git ls-tree -r` (it
/// recurses to zero blob lines), so an empty collection would otherwise be
/// indistinguishable, to git's tooling, from a field that never existed.
#[test]
fn empty_vec_is_marker_tree() {
    let (root_id, store) = serialize(&WithVec { items: vec![] }).expect("serialize should succeed");

    let (mode, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    assert_eq!(mode, EntryKind::Tree, "empty Vec field must be a tree");
    let entries = tree_entries(&store, &items_id);
    assert_eq!(
        entries.len(),
        1,
        "empty Vec must hold exactly the presence marker"
    );
    assert_eq!(
        entries[0].filename, "none",
        "the marker entry must be named \"none\""
    );
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store
            .get_blob(&entries[0].oid)
            .expect("read marker")
            .expect("marker blob"),
        b"",
        "the marker must point to the empty blob"
    );
}

// --- Array ---

/// A fixed-size array field is encoded as a tree holding only its elements.
#[test]
fn array_field_is_tree() {
    let (root_id, store) = serialize(&WithArray {
        values: [1, 2, 3, 4],
    })
    .expect("serialize should succeed");

    let (mode, arr_id) = get_tree_entry_mode(&store, &root_id, "values");
    assert_eq!(mode, EntryKind::Tree, "array field must be a tree");
    assert_eq!(
        tree_entries(&store, &arr_id).len(),
        4,
        "array of length 4 should have 4 entries"
    );
}

// --- Map ---

/// A HashMap field is encoded as a tree holding only its entries.
#[test]
fn map_field_is_tree() {
    let mut table = HashMap::new();
    table.insert("a".to_string(), "1".to_string());
    table.insert("b".to_string(), "2".to_string());

    let (root_id, store) = serialize(&WithMap { table }).expect("serialize should succeed");

    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree, "Map field must be a tree");
    assert_eq!(
        tree_entries(&store, &map_id).len(),
        2,
        "map with 2 entries should have 2 entries"
    );
}

/// An empty map serializes to the presence-marker tree, not a literal empty
/// tree — the same reasoning as `empty_vec_is_marker_tree`.
#[test]
fn empty_map_is_marker_tree() {
    let (root_id, store) = serialize(&WithMap {
        table: HashMap::new(),
    })
    .expect("serialize should succeed");

    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree, "empty Map field must be a tree");
    let entries = tree_entries(&store, &map_id);
    assert_eq!(
        entries.len(),
        1,
        "empty Map must hold exactly the presence marker"
    );
    assert_eq!(
        entries[0].filename, "none",
        "the marker entry must be named \"none\""
    );
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store
            .get_blob(&entries[0].oid)
            .expect("read marker")
            .expect("marker blob"),
        b"",
        "the marker must point to the empty blob"
    );
}

/// String keys are escaped, ordered by their encoded names, and resolve to values.
#[test]
fn map_entries_are_named_by_escaped_keys() {
    let table: HashMap<String, String> = [
        ("a", "1"),
        ("a/b", "slash"),
        ("none", "reserved"),
        ("some", "option"),
        ("_", "underscore"),
        ("", "empty"),
        (".value", "dot"),
        ("naïve", "unicode"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let input = WithMap { table };
    let (root_id, store) = serialize(&input).expect("serialize should succeed");
    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree);
    let entries = tree_entries(&store, &map_id);
    let expected: &[(&str, &[u8])] = &[
        ("%", b"empty\n"),
        ("%.value", b"dot\n"),
        ("%a%2Fb", b"slash\n"),
        ("%na%C3%AFve", b"unicode\n"),
        ("%none", b"reserved\n"),
        ("%some", b"option\n"),
        ("_", b"underscore\n"),
        ("a", b"1\n"),
    ];
    assert_eq!(entries.len(), expected.len());
    for (entry, (name, value)) in entries.iter().zip(expected) {
        assert_eq!(entry.filename, *name);
        assert_eq!(entry.mode.kind(), EntryKind::Blob);
        assert_eq!(
            store
                .get_blob(&entry.oid)
                .expect("read object")
                .expect("value blob"),
            *value
        );
    }
    assert_eq!(
        deserialize::<WithMap>(&root_id, &store).expect("deserialize"),
        input
    );
}

/// Integer-keyed maps are opaque canonical CBOR, not name-keyed trees.
#[test]
fn map_with_int_keys_is_cbor() {
    let input = WithIntMap {
        table: HashMap::from([(42, "x".to_string())]),
    };
    let (root_id, store) = serialize(&input).expect("serialize should succeed");
    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Blob, "non-string map must be a CBOR leaf");
    assert_eq!(
        store
            .get_blob(&map_id)
            .expect("read object")
            .expect("CBOR blob"),
        [0xa1, 0x18, 0x2a, 0x61, b'x'],
        "{{ 42: \"x\" }} in core-deterministic CBOR, with no newline"
    );
    assert_eq!(
        schema_of::<HashMap<u32, String>>().expect("schema").root,
        Node::Cbor
    );
    assert_eq!(
        deserialize::<WithIntMap>(&root_id, &store).expect("deserialize"),
        input
    );
}

/// Empty non-string-keyed maps are still CBOR leaves, not native map markers.
#[test]
fn empty_int_key_map_is_empty_cbor_map() {
    let input = WithIntMap {
        table: HashMap::new(),
    };
    let (root, store) = serialize(&input).expect("serialize");
    let (mode, oid) = get_tree_entry_mode(&store, &root, "table");
    assert_eq!(mode, EntryKind::Blob);
    assert_eq!(
        store.get_blob(&oid).expect("read object").expect("blob"),
        [0xa0]
    );
    assert_eq!(
        deserialize::<WithIntMap>(&root, &store).expect("deserialize"),
        input
    );
}

/// Map insertion order does not affect the serialized tree: git sorts tree entries
/// by name, so two maps with the same pairs produce the same root object ID.
#[test]
fn map_insertion_order_is_irrelevant() {
    let mut a = HashMap::new();
    a.insert("alpha".to_string(), "1".to_string());
    a.insert("beta".to_string(), "2".to_string());
    a.insert("gamma".to_string(), "3".to_string());

    let mut b = HashMap::new();
    b.insert("gamma".to_string(), "3".to_string());
    b.insert("alpha".to_string(), "1".to_string());
    b.insert("beta".to_string(), "2".to_string());

    let (id_a, _) = serialize(&WithMap { table: a }).expect("serialize should succeed");
    let (id_b, _) = serialize(&WithMap { table: b }).expect("serialize should succeed");
    assert_eq!(
        id_a, id_b,
        "maps with identical pairs must serialize identically regardless of insertion order"
    );
}

/// A map keyed by a smart pointer to a scalar (`Arc<str>`) is name-keyed by
/// the textual form of the collapsed key shape (`str`'s own textual form),
/// not treated as composite merely because the key's own static shape is
/// `Def::Pointer`. This is the encoder-side half of the map-key transparency
/// collapse: `schema_of::<HashMap<Arc<str>, u32>>()` classifies the same key
/// scalar (covered in `schema_driven.rs`), and both sides must agree on what
/// actually got written.
#[test]
fn map_with_smart_pointer_scalar_keys_is_name_keyed() {
    let mut table: HashMap<Arc<str>, u32> = HashMap::new();
    table.insert(Arc::from("hello"), 5);

    let (root_id, store) = serialize(&WithArcStrKeyMap {
        table: table.clone(),
    })
    .expect("serialize should succeed");

    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree, "Map field must be a tree");

    let (vmode, v_id) = get_tree_entry_mode(&store, &map_id, "hello");
    assert_eq!(
        vmode,
        EntryKind::Blob,
        "an Arc<str> key must use its collapsed string value as an escaped entry name"
    );
    assert_eq!(
        store
            .get_blob(&v_id)
            .expect("read object")
            .expect("value blob in store"),
        b"5\n",
        "name-keyed entry must resolve directly to the value"
    );

    let got: WithArcStrKeyMap = deserialize(&root_id, &store).expect("deserialize should succeed");
    assert_eq!(got.table, table, "Arc<str>-keyed map must round-trip");
}

// --- composite map keys ---

/// A struct-keyed map is one CBOR blob carrying both key fields and the value.
#[test]
fn map_with_composite_keys_is_cbor() {
    let table = HashMap::from([(Coord { x: 1, y: 2 }, "a".to_string())]);
    let input = WithCompositeKeyMap { table };
    let (root_id, store) = serialize(&input).expect("serialize");
    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(
        mode,
        EntryKind::Blob,
        "composite-keyed map must be a CBOR leaf"
    );
    // { { "x": 1, "y": 2 }: "a" }, with bytewise-ordered CBOR map keys.
    assert_eq!(
        store
            .get_blob(&map_id)
            .expect("read object")
            .expect("CBOR blob"),
        [0xa1, 0xa2, 0x61, b'x', 0x01, 0x61, b'y', 0x02, 0x61, b'a']
    );
    assert_eq!(
        schema_of::<HashMap<Coord, String>>().expect("schema").root,
        Node::Cbor
    );
    assert_eq!(
        deserialize::<WithCompositeKeyMap>(&root_id, &store).expect("deserialize"),
        input
    );
}

/// A composite-keyed map round-trips, and insertion order does not affect identity.
#[test]
fn map_with_composite_keys_roundtrips_order_independently() {
    let mut a = HashMap::new();
    a.insert(Coord { x: 1, y: 2 }, "a".to_string());
    a.insert(Coord { x: 3, y: 4 }, "b".to_string());

    let mut b = HashMap::new();
    b.insert(Coord { x: 3, y: 4 }, "b".to_string());
    b.insert(Coord { x: 1, y: 2 }, "a".to_string());

    let (id_a, store) = serialize(&WithCompositeKeyMap { table: a.clone() }).expect("serialize");
    let (id_b, _) = serialize(&WithCompositeKeyMap { table: b }).expect("serialize");
    assert_eq!(
        id_a, id_b,
        "composite-keyed maps must be content-addressed independent of insertion order"
    );

    let got: WithCompositeKeyMap = deserialize(&id_a, &store).expect("deserialize");
    assert_eq!(got.table, a, "composite-keyed map must round-trip");
}

/// Adding a native string-map entry leaves all existing entry names and oids
/// unchanged, even when the new escaped name sorts before them.
#[test]
fn adding_a_string_pair_does_not_rename_existing_pairs() {
    let small = HashMap::from([
        ("alpha".to_string(), "a".to_string()),
        ("beta".to_string(), "b".to_string()),
    ]);
    let (small_root, small_store) = serialize(&WithMap {
        table: small.clone(),
    })
    .expect("serialize");
    let (_, small_map) = get_tree_entry_mode(&small_store, &small_root, "table");
    let small_entries = tree_entries(&small_store, &small_map);

    let mut grown = small;
    grown.insert("a/b".to_string(), "z".to_string());
    let (grown_root, grown_store) = serialize(&WithMap {
        table: grown.clone(),
    })
    .expect("serialize");
    let (_, grown_map) = get_tree_entry_mode(&grown_store, &grown_root, "table");
    let grown_entries = tree_entries(&grown_store, &grown_map);
    assert_eq!(grown_entries.len(), small_entries.len() + 1);
    assert_eq!(grown_entries[0].filename, "%a%2Fb");
    for entry in &small_entries {
        assert!(
            grown_entries
                .iter()
                .any(|g| g.filename == entry.filename && g.oid == entry.oid),
            "existing pair entry {} must keep its name and target",
            entry.filename
        );
    }
    assert_eq!(
        deserialize::<WithMap>(&grown_root, &grown_store)
            .expect("deserialize")
            .table,
        grown
    );
}

/// A composite-key map has a single changed leaf when a pair is added, and
/// decoding the new leaf preserves all old pairs as well as the new one.
#[test]
fn adding_a_composite_pair_replaces_only_the_cbor_leaf() {
    let small = HashMap::from([
        (Coord { x: 5, y: 5 }, "a".to_string()),
        (Coord { x: 6, y: 6 }, "b".to_string()),
    ]);
    let (small_root, small_store) = serialize(&WithCompositeKeyMap {
        table: small.clone(),
    })
    .expect("serialize");
    let small_entries = tree_entries(&small_store, &small_root);
    let mut grown = small;
    grown.insert(Coord { x: -9, y: -9 }, "z".to_string());
    let (grown_root, grown_store) = serialize(&WithCompositeKeyMap {
        table: grown.clone(),
    })
    .expect("serialize");
    let grown_entries = tree_entries(&grown_store, &grown_root);
    assert_eq!(small_entries.len(), 1);
    assert_eq!(grown_entries.len(), 1);
    assert_eq!(small_entries[0].filename, "table");
    assert_eq!(grown_entries[0].filename, "table");
    assert_eq!(small_entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(grown_entries[0].mode.kind(), EntryKind::Blob);
    assert_ne!(small_entries[0].oid, grown_entries[0].oid);
    assert_eq!(
        deserialize::<WithCompositeKeyMap>(&grown_root, &grown_store)
            .expect("deserialize")
            .table,
        grown
    );
}

/// Old empty-tree and underscore-marker encodings are not valid empty native
/// collections. `_` remains a valid map key when its value is a text leaf.
#[test]
fn native_collections_reject_legacy_empty_encodings() {
    let store = ObjectStore::default();
    let empty_blob = store.write_buf(Kind::Blob, b"").expect("write empty blob");
    let text_blob = store.write_buf(Kind::Blob, b"\n").expect("write text leaf");
    let tree = |entries| store.write(&Tree { entries }).expect("write tree");
    let entry = |name: &str, kind: EntryKind, oid| TreeEntry {
        mode: EntryMode::from(kind),
        filename: name.into(),
        oid,
    };
    let empty = tree(vec![]);
    let old_marker = tree(vec![entry("_", EntryKind::Blob, empty_blob)]);
    let wrong_marker = tree(vec![entry("none", EntryKind::Blob, text_blob)]);
    for root in [empty, old_marker, wrong_marker] {
        assert!(
            deserialize::<Vec<i64>>(&root, &store).is_err(),
            "invalid vec tree {root}"
        );
        assert!(
            deserialize::<HashMap<String, String>>(&root, &store).is_err(),
            "invalid map tree {root}"
        );
    }
    let underscore = tree(vec![entry("_", EntryKind::Blob, text_blob)]);
    assert_eq!(
        deserialize::<HashMap<String, String>>(&underscore, &store).expect("underscore key"),
        HashMap::from([("_".to_string(), "".to_string())])
    );
}

/// Non-string maps must be blobs; legacy integer-name and composite-pair trees
/// cannot be read as CBOR even when their tree contents are well formed.
#[test]
fn non_string_maps_reject_legacy_tree_layouts() {
    let store = ObjectStore::default();
    let leaf = |bytes: &[u8]| store.write_buf(Kind::Blob, bytes).expect("write blob");
    let tree = |entries| store.write(&Tree { entries }).expect("write tree");
    let entry = |name: &str, kind: EntryKind, oid| TreeEntry {
        mode: EntryMode::from(kind),
        filename: name.into(),
        oid,
    };
    let integer_map = tree(vec![entry("42", EntryKind::Blob, leaf(b"x\n"))]);
    let integer_root = tree(vec![entry("table", EntryKind::Tree, integer_map)]);
    assert!(deserialize::<WithIntMap>(&integer_root, &store).is_err());

    let key = tree(vec![
        entry("x", EntryKind::Blob, leaf(b"1\n")),
        entry("y", EntryKind::Blob, leaf(b"2\n")),
    ]);
    let pair = tree(vec![
        entry("k", EntryKind::Tree, key),
        entry("v", EntryKind::Blob, leaf(b"a\n")),
    ]);
    let composite_map = tree(vec![entry(&pair.to_string(), EntryKind::Tree, pair)]);
    let composite_root = tree(vec![entry("table", EntryKind::Tree, composite_map)]);
    assert!(deserialize::<WithCompositeKeyMap>(&composite_root, &store).is_err());
}

/// Independent CBOR fixture generator for the small generated coordinate maps.
fn expected_coord_map_cbor(entries: &std::collections::BTreeMap<Coord, String>) -> Vec<u8> {
    fn small_int(value: i32) -> u8 {
        assert!((-20..20).contains(&value));
        if value >= 0 {
            value as u8
        } else {
            0x20 + (-1 - value) as u8
        }
    }
    let mut pairs: Vec<_> = entries
        .iter()
        .map(|(key, value)| {
            assert!(value.len() < 24);
            let key = vec![
                0xa2,
                0x61,
                b'x',
                small_int(key.x),
                0x61,
                b'y',
                small_int(key.y),
            ];
            let mut value_bytes = vec![0x60 + value.len() as u8];
            value_bytes.extend_from_slice(value.as_bytes());
            (key, value_bytes)
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(pairs.len() < 24);
    let mut result = vec![0xa0 + pairs.len() as u8];
    for (key, value) in pairs {
        result.extend(key);
        result.extend(value);
    }
    result
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    #[test]
    fn scalar_maps_are_deterministic_across_independent_stores(
        entries in proptest::collection::btree_map("[a-z]{1,6}", "[a-z]{0,8}", 0..6),
    ) {
        let value = WithMap { table: entries.into_iter().collect() };
        let (first, _) = serialize(&value).expect("serialize");
        let (second, _) = serialize(&value).expect("serialize");
        prop_assert_eq!(first, second);
    }

    #[test]
    fn scalar_map_insertion_order_does_not_change_root_id(
        entries in proptest::collection::btree_map("[a-z]{1,6}", "[a-z]{0,8}", 1..6),
    ) {
        let pairs: Vec<_> = entries.into_iter().collect();
        let forward = pairs.iter().cloned().collect::<HashMap<_, _>>();
        let reverse = pairs.iter().rev().cloned().collect::<HashMap<_, _>>();
        let (first, _) = serialize(&WithMap { table: forward }).expect("serialize");
        let (second, _) = serialize(&WithMap { table: reverse }).expect("serialize");
        prop_assert_eq!(first, second);
    }

    #[test]
    fn scalar_map_keys_are_valid_tree_entry_names(
        entries in proptest::collection::btree_map("[a-z][a-z0-9]{0,5}", "[a-z]{0,8}", 1..6),
    ) {
        let expected: HashMap<_, _> = entries.into_iter().collect();
        let (root, store) = serialize(&WithMap { table: expected.clone() }).expect("serialize");
        let (_, map_id) = get_tree_entry_mode(&store, &root, "table");
        let names: Vec<_> = tree_entries(&store, &map_id)
            .into_iter()
            .map(|entry| entry.filename.to_string())
            .collect();
        let mut expected_names: Vec<_> = expected.keys().map(|key| match key.as_str() {
            "none" | "some" => format!("%{key}"),
            _ => key.clone(),
        }).collect();
        expected_names.sort();
        prop_assert_eq!(names.clone(), expected_names);
        prop_assert!(names.iter().all(|name| !name.contains('/') && name != "none" && name != "some"));
    }

    #[test]
    fn composite_map_cbor_is_deterministic(
        entries in proptest::collection::btree_map(
            (-20i32..20, -20i32..20).prop_map(|(x, y)| Coord { x, y }),
            "[a-z]{1,8}",
            1..6,
        ),
    ) {
        let pairs: Vec<_> = entries.into_iter().collect();
        let forward = pairs.iter().cloned().collect::<HashMap<_, _>>();
        let reverse = pairs.iter().rev().cloned().collect::<HashMap<_, _>>();
        let (first, _) = serialize(&WithCompositeKeyMap { table: forward }).expect("serialize");
        let (second, _) = serialize(&WithCompositeKeyMap { table: reverse }).expect("serialize");
        prop_assert_eq!(first, second);
    }

    #[test]
    fn composite_map_entries_are_canonical_cbor(
        entries in proptest::collection::btree_map(
            (-20i32..20, -20i32..20).prop_map(|(x, y)| Coord { x, y }),
            "[a-z]{1,8}",
            1..6,
        ),
    ) {
        let bytes = expected_coord_map_cbor(&entries);
        let table: HashMap<_, _> = entries.into_iter().collect();
        let (root, store) = serialize(&WithCompositeKeyMap { table: table.clone() }).expect("serialize");
        let (mode, map_id) = get_tree_entry_mode(&store, &root, "table");
        prop_assert_eq!(mode, EntryKind::Blob);
        let actual = store.get_blob(&map_id).expect("read object").expect("CBOR blob");
        prop_assert_eq!(actual, bytes);
        let decoded: WithCompositeKeyMap = deserialize(&root, &store).expect("deserialize");
        prop_assert_eq!(&decoded.table, &table);
        prop_assert_eq!(serialize(&decoded).expect("re-encode").0, root);
    }
}

// --- duplicate composite keys ---

/// Distinct Rust NaN keys encode to the same canonical CBOR key and must be
/// rejected on write, rather than silently picking a value.
#[test]
fn duplicate_composite_keys_are_refused_on_write() {
    let mut table = HashMap::new();
    table.insert(FloatKey { x: f64::NAN }, "a".to_string());
    table.insert(FloatKey { x: f64::NAN }, "b".to_string());
    assert_eq!(table.len(), 2, "NaN keys are distinct to the map");

    let err = serialize(&WithFloatKeyMap { table }).expect_err("duplicate key must be refused");
    assert!(
        matches!(
            &err,
            SerializeError::Reflect(message)
                if message.contains("cannot encode")
                    && message.contains("duplicate CBOR map key")
        ),
        "expected a duplicate CBOR key error, got {err:?}"
    );
}

/// A foreign CBOR blob with two copies of the same composite key is invalid.
#[test]
fn duplicate_composite_keys_are_refused_on_read() {
    let store = ObjectStore::default();
    // { {"x": 1, "y": 1}: "a", {"x": 1, "y": 1}: "b" }.
    let bytes = [
        0xa2, 0xa2, 0x61, b'x', 0x01, 0x61, b'y', 0x01, 0x61, b'a', 0xa2, 0x61, b'x', 0x01, 0x61,
        b'y', 0x01, 0x61, b'b',
    ];
    let blob = store
        .write_buf(Kind::Blob, &bytes)
        .expect("write CBOR blob");
    let root = store
        .write(&Tree {
            entries: vec![TreeEntry {
                mode: EntryMode::from(EntryKind::Blob),
                filename: "table".into(),
                oid: blob,
            }],
        })
        .expect("write root tree");
    let err = deserialize::<WithCompositeKeyMap>(&root, &store)
        .expect_err("duplicate keys must be refused");
    assert!(
        matches!(
            &err,
            DeserializeError::Reflect(message)
                if message.contains("invalid CBOR blob")
                    && message.contains("duplicate or out-of-order CBOR map key")
        ),
        "expected a duplicate CBOR key error, got {err:?}"
    );
}
