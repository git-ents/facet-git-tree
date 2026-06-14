//! Integration tests for built-in collection type serialization.
//!
//! Covers spec requirement:
//!   serialization.design.trees.collections
//!     — Array, Vec, and Map must be provided as built-in Facet types
//!     — each collection must carry its own `.schema` sentinel entry
//!
//! Ordinal entry naming for sequence collections is covered in `ordinals.rs`.

use std::collections::HashMap;

use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{
    WithArray, WithMap, WithVec, assert_schema_sentinel, get_tree_entry_mode, non_sentinel_entries,
};

// --- Vec ---

/// A Vec field is encoded as a tree (not a blob), with its own `.schema` sentinel.
#[test]
#[ignore = "serialization not yet implemented"]
fn vec_field_is_tree_with_schema() {
    let (root_id, store) = serialize(&WithVec {
        items: vec![1, 2, 3],
    })
    .expect("serialize should succeed");

    let (mode, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    assert_eq!(mode, EntryKind::Tree, "Vec field must be a tree");

    assert_schema_sentinel(&store, &items_id, "Vec field");
}

/// An empty Vec also serializes to a tree with a `.schema` sentinel.
#[test]
#[ignore = "serialization not yet implemented"]
fn empty_vec_has_schema() {
    let (root_id, store) = serialize(&WithVec { items: vec![] }).expect("serialize should succeed");

    let (mode, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    assert_eq!(mode, EntryKind::Tree, "empty Vec field must be a tree");
    assert_schema_sentinel(&store, &items_id, "empty Vec field");
}

/// Vec elements are accessible as individual entries within the collection tree.
#[test]
#[ignore = "serialization not yet implemented"]
fn vec_elements_accessible() {
    let (root_id, store) = serialize(&WithVec {
        items: vec![10, 20],
    })
    .expect("serialize should succeed");

    let (_, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    assert_eq!(
        non_sentinel_entries(&store, &items_id).len(),
        2,
        "Vec with 2 elements should have 2 non-sentinel entries"
    );
}

// --- Array ---

/// A fixed-size array field is encoded as a tree with its own `.schema` sentinel.
#[test]
#[ignore = "serialization not yet implemented"]
fn array_field_is_tree_with_schema() {
    let (root_id, store) = serialize(&WithArray {
        values: [1, 2, 3, 4],
    })
    .expect("serialize should succeed");

    let (mode, arr_id) = get_tree_entry_mode(&store, &root_id, "values");
    assert_eq!(mode, EntryKind::Tree, "array field must be a tree");
    assert_schema_sentinel(&store, &arr_id, "array field");
}

/// Array elements are accessible within the collection tree.
#[test]
#[ignore = "serialization not yet implemented"]
fn array_elements_accessible() {
    let (root_id, store) = serialize(&WithArray {
        values: [10, 20, 30, 40],
    })
    .expect("serialize should succeed");

    let (_, arr_id) = get_tree_entry_mode(&store, &root_id, "values");
    assert_eq!(
        non_sentinel_entries(&store, &arr_id).len(),
        4,
        "array of length 4 should have 4 non-sentinel entries"
    );
}

// --- Map ---

/// A HashMap field is encoded as a tree with its own `.schema` sentinel.
#[test]
#[ignore = "serialization not yet implemented"]
fn map_field_is_tree_with_schema() {
    let mut table = HashMap::new();
    table.insert("a".to_string(), "1".to_string());
    table.insert("b".to_string(), "2".to_string());

    let (root_id, store) = serialize(&WithMap { table }).expect("serialize should succeed");

    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree, "Map field must be a tree");
    assert_schema_sentinel(&store, &map_id, "Map field");
}

/// An empty map serializes to a tree with a `.schema` sentinel.
#[test]
#[ignore = "serialization not yet implemented"]
fn empty_map_has_schema() {
    let (root_id, store) = serialize(&WithMap {
        table: HashMap::new(),
    })
    .expect("serialize should succeed");

    let (mode, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(mode, EntryKind::Tree, "empty Map field must be a tree");
    assert_schema_sentinel(&store, &map_id, "empty Map field");
}

/// Map insertion order does not affect the serialized tree: git sorts tree entries
/// by name, so two maps with the same pairs produce the same root object ID.
#[test]
#[ignore = "serialization not yet implemented"]
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

/// Map entries are accessible as individual entries within the collection tree.
#[test]
#[ignore = "serialization not yet implemented"]
fn map_entries_accessible() {
    let mut table = HashMap::new();
    table.insert("key1".to_string(), "100".to_string());
    table.insert("key2".to_string(), "200".to_string());

    let (root_id, store) = serialize(&WithMap { table }).expect("serialize should succeed");

    let (_, map_id) = get_tree_entry_mode(&store, &root_id, "table");
    assert_eq!(
        non_sentinel_entries(&store, &map_id).len(),
        2,
        "map with 2 entries should have 2 non-sentinel entries"
    );
}
