//! Integration tests for built-in collection type serialization.
//!
//! Covers spec requirement:
//!   serialization.design.trees.collections
//!     — Array, Vec, and Map are encoded as Git trees
//!     — no type marker is recorded; element and key types come from the Facet type
//!
//! Ordinal entry naming for sequence collections is covered in `ordinals.rs`.

use std::collections::HashMap;

use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{WithArray, WithMap, WithVec, get_tree_entry_mode, tree_entries};

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

/// An empty Vec serializes to an empty tree.
#[test]
fn empty_vec_is_empty_tree() {
    let (root_id, store) = serialize(&WithVec { items: vec![] }).expect("serialize should succeed");

    let (mode, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    assert_eq!(mode, EntryKind::Tree, "empty Vec field must be a tree");
    assert!(
        tree_entries(&store, &items_id).is_empty(),
        "empty Vec must have no entries"
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

/// An empty map serializes to an empty tree.
#[test]
fn empty_map_is_empty_tree() {
    let (root_id, store) = serialize(&WithMap {
        table: HashMap::new(),
    })
    .expect("serialize should succeed");

    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree, "empty Map field must be a tree");
    assert!(
        tree_entries(&store, &map_id).is_empty(),
        "empty Map must have no entries"
    );
}

/// A map entry is named by the textual form of its key and resolves to its value.
#[test]
fn map_entry_named_by_key() {
    let mut table = HashMap::new();
    table.insert("a".to_string(), "1".to_string());

    let (root_id, store) = serialize(&WithMap { table }).expect("serialize should succeed");

    let (_, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    let (mode, value_id) = get_tree_entry_mode(&store, &map_id, "a");
    assert_eq!(mode, EntryKind::Blob, "map value must be a leaf blob");
    assert_eq!(
        store.get_blob(&value_id).expect("value blob in store"),
        b"1",
        "map entry named by key must resolve to the value"
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
