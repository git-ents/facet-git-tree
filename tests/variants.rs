//! Integration tests for enum/variant serialization.
//!
//! Covers spec requirement:
//!   serialization.design.trees.variants
//!     — an enum is a tree with exactly one entry, externally tagged: the entry
//!       name is the active variant, its value encodes the payload
//!     — unit variant → empty tree; struct variant → fields named by field name;
//!       tuple variant → fields named by zero-padded zero-based index (0000, …)

use facet::Facet;
use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{find_entry, roundtrip, tree_entries};

// --- test types ---

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum Shape {
    Unit,
    Circle { radius: f64 },
    Pair(i32, i32),
}

// --- structure ---

/// An enum value is a tree with exactly one entry, named after the active variant.
#[test]
#[ignore = "serialization not yet implemented"]
fn enum_is_single_entry_tree() {
    let (root_id, store) = serialize(&Shape::Circle { radius: 1.0 }).expect("serialize ok");
    let entries = tree_entries(&store, &root_id);
    assert_eq!(
        entries.len(),
        1,
        "enum must be a tree with exactly one (variant-named) entry, got {entries:?}"
    );
    assert_eq!(
        entries[0].filename, "Circle",
        "the single entry must be named after the active variant"
    );
}

/// The active variant for a unit variant is recorded the same way: a sole entry
/// named after it, resolving to an empty tree (no payload).
#[test]
#[ignore = "serialization not yet implemented"]
fn unit_variant_is_named_empty_tree() {
    let (root_id, store) = serialize(&Shape::Unit).expect("serialize ok");
    let entry = find_entry(&store, &root_id, "Unit");
    assert_eq!(
        entry.mode.kind(),
        EntryKind::Tree,
        "a unit variant's payload must be a tree"
    );
    assert!(
        tree_entries(&store, &entry.oid).is_empty(),
        "a unit variant's payload tree must be empty"
    );
}

/// A tuple variant's sole entry is named after it.
#[test]
#[ignore = "serialization not yet implemented"]
fn tuple_variant_is_named() {
    let (root_id, store) = serialize(&Shape::Pair(1, 2)).expect("serialize ok");
    let _ = find_entry(&store, &root_id, "Pair");
}

/// Struct-variant fields are encoded under the variant entry, named by field name.
#[test]
#[ignore = "serialization not yet implemented"]
fn struct_variant_fields_named_by_field() {
    let (root_id, store) = serialize(&Shape::Circle { radius: 2.5 }).expect("serialize ok");
    let circle = find_entry(&store, &root_id, "Circle");
    let radius = find_entry(&store, &circle.oid, "radius");
    assert_eq!(
        radius.mode.kind(),
        EntryKind::Blob,
        "`radius` must be a leaf blob"
    );
}

/// Tuple-variant fields are encoded under the variant entry, named by their
/// zero-padded, zero-based index (`0000`, `0001`, …).
#[test]
#[ignore = "serialization not yet implemented"]
fn tuple_variant_fields_named_by_index() {
    let (root_id, store) = serialize(&Shape::Pair(7, 13)).expect("serialize ok");
    let pair = find_entry(&store, &root_id, "Pair");
    let _ = find_entry(&store, &pair.oid, "0000");
    let _ = find_entry(&store, &pair.oid, "0001");
}

// --- equality ---

/// Distinct variants of the same enum produce distinct root object IDs.
#[test]
#[ignore = "serialization not yet implemented"]
fn distinct_variants_differ() {
    let (unit, _) = serialize(&Shape::Unit).expect("serialize ok");
    let (circle, _) = serialize(&Shape::Circle { radius: 0.0 }).expect("serialize ok");
    assert_ne!(
        unit, circle,
        "different active variants must produce different object IDs"
    );
}

// --- roundtrip ---

#[test]
#[ignore = "serialization not yet implemented"]
fn unit_variant_roundtrip() {
    assert_eq!(roundtrip(Shape::Unit), Shape::Unit);
}

#[test]
#[ignore = "serialization not yet implemented"]
fn struct_variant_roundtrip() {
    assert_eq!(
        roundtrip(Shape::Circle { radius: 3.5 }),
        Shape::Circle { radius: 3.5 }
    );
}

#[test]
#[ignore = "serialization not yet implemented"]
fn tuple_variant_roundtrip() {
    assert_eq!(roundtrip(Shape::Pair(-1, 99)), Shape::Pair(-1, 99));
}
