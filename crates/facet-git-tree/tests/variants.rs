//! Integration tests for enum/variant serialization.
//!
//! Covers spec requirement:
//!   serialization.design.trees.variants
//!     — enums are schema objects with a `.variant` sentinel naming the active variant
//!     — struct-variant fields are named by field name
//!     — tuple-variant fields are named by zero-padded zero-based index (0000, 0001, …)

use facet::Facet;
use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{find_entry, non_sentinel_entries, roundtrip};

// --- test types ---

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum Shape {
    Unit,
    Circle { radius: f64 },
    Pair(i32, i32),
}

// --- structure ---

/// An enum value is a schema object: its root is a tree with a `.schema` sentinel.
#[test]
#[ignore = "serialization not yet implemented"]
fn enum_is_schema_object() {
    let (root_id, store) = serialize(&Shape::Circle { radius: 1.0 }).expect("serialize ok");
    let schema = find_entry(&store, &root_id, ".schema");
    assert_eq!(
        schema.mode.kind(),
        EntryKind::Tree,
        "`.schema` must be a tree"
    );
}

/// The active variant's name is recorded in a `.variant` blob.
#[test]
#[ignore = "serialization not yet implemented"]
fn variant_name_recorded_in_sentinel() {
    let (root_id, store) = serialize(&Shape::Circle { radius: 1.0 }).expect("serialize ok");
    let variant = find_entry(&store, &root_id, ".variant");
    assert_eq!(
        variant.mode.kind(),
        EntryKind::Blob,
        "`.variant` sentinel must be a blob"
    );
    let bytes = store
        .get_blob(&variant.oid)
        .expect("`.variant` blob must be in store");
    assert_eq!(
        bytes, b"Circle",
        "`.variant` must hold the active variant name"
    );
}

/// The `.variant` name is recorded for a unit variant too (no fields to imply it).
#[test]
#[ignore = "serialization not yet implemented"]
fn unit_variant_name_recorded() {
    let (root_id, store) = serialize(&Shape::Unit).expect("serialize ok");
    let variant = find_entry(&store, &root_id, ".variant");
    let bytes = store
        .get_blob(&variant.oid)
        .expect("`.variant` blob in store");
    assert_eq!(bytes, b"Unit", "`.variant` must name the unit variant");
}

/// The `.variant` name is recorded for a tuple variant too.
#[test]
#[ignore = "serialization not yet implemented"]
fn tuple_variant_name_recorded() {
    let (root_id, store) = serialize(&Shape::Pair(1, 2)).expect("serialize ok");
    let variant = find_entry(&store, &root_id, ".variant");
    let bytes = store
        .get_blob(&variant.oid)
        .expect("`.variant` blob in store");
    assert_eq!(bytes, b"Pair", "`.variant` must name the tuple variant");
}

/// Struct-variant fields are encoded as entries named by their field name.
#[test]
#[ignore = "serialization not yet implemented"]
fn struct_variant_fields_named_by_field() {
    let (root_id, store) = serialize(&Shape::Circle { radius: 2.5 }).expect("serialize ok");
    let radius = find_entry(&store, &root_id, "radius");
    assert_eq!(
        radius.mode.kind(),
        EntryKind::Blob,
        "`radius` must be a leaf blob"
    );
}

/// Tuple-variant fields are encoded as entries named by their zero-padded,
/// zero-based index (`0000`, `0001`, …).
#[test]
#[ignore = "serialization not yet implemented"]
fn tuple_variant_fields_named_by_index() {
    let (root_id, store) = serialize(&Shape::Pair(7, 13)).expect("serialize ok");
    let _ = find_entry(&store, &root_id, "0000");
    let _ = find_entry(&store, &root_id, "0001");
}

/// A unit variant carries only sentinels — no field entries.
#[test]
#[ignore = "serialization not yet implemented"]
fn unit_variant_has_no_fields() {
    let (root_id, store) = serialize(&Shape::Unit).expect("serialize ok");
    let fields = non_sentinel_entries(&store, &root_id);
    assert!(
        fields.is_empty(),
        "unit variant must have no field entries, got {fields:?}"
    );
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
