//! Integration tests for enum/variant serialization.
//!
//! Covers spec requirement:
//!   serialization.design.trees.variants
//!     — a unit variant is a bare blob holding the variant name (its entire
//!       information content), so it appears as ordinary content to
//!       `git diff`/`ls-tree -r`; every other variant is a tree with exactly
//!       one entry, externally tagged: the entry name is the active variant,
//!       its value encodes the payload — struct variant fields are named by
//!       escaped field name; a one-field tuple variant encodes the field
//!       directly, and multi-field tuple variants use three-digit lowercase
//!       hexadecimal ordinals (000, 001, …).

use facet::Facet;
use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{find_entry, roundtrip, tree_entries};

// --- test types ---

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum Shape {
    Unit,
    Circle {
        radius: f64,
    },
    Pair(i32, i32),
    Text(String),
    Numbers(Vec<i32>),
    Bytes(Vec<u8>),
    #[facet(rename = "none")]
    Reserved,
    #[facet(rename = "a/b")]
    Renamed {
        #[facet(rename = "some")]
        value: i32,
    },
}

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Facet, PartialEq)]
struct WithPriority {
    priority: Priority,
}

// --- structure ---

/// A non-unit enum value is a tree with exactly one entry, named after the active variant.
#[test]
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

/// A unit variant collapses to a bare blob holding the variant name text —
/// not a tree named after it — as a standalone value.
#[test]
fn unit_variant_is_a_bare_name_blob() {
    let (root_id, store) = serialize(&Shape::Unit).expect("serialize ok");
    assert_eq!(
        store
            .get_blob(&root_id)
            .expect("read object")
            .expect("unit variant must serialize to a blob"),
        b"Unit\n",
        "a unit variant's entire encoding is a blob holding the variant name, \
         plus the mandatory trailing newline for text leaves"
    );
}

/// The same collapse holds when the unit variant is a struct field: the
/// field's own tree entry becomes a blob, not a tree wrapping an empty tree.
/// This is what makes `priority: Low` → `High` show up as a `git diff`
/// (`-Low`/`+High`) and a `priority` line in `git ls-tree -r`, instead of a
/// tree-entry rename with no blob content on either side.
#[test]
fn unit_variant_field_is_a_bare_name_blob() {
    let (root_id, store) = serialize(&WithPriority {
        priority: Priority::High,
    })
    .expect("serialize ok");
    let entry = find_entry(&store, &root_id, "priority");
    assert_eq!(
        entry.mode.kind(),
        EntryKind::Blob,
        "a unit-variant field's entry must be a blob, not a tree"
    );
    assert_eq!(
        store
            .get_blob(&entry.oid)
            .expect("read object")
            .expect("blob"),
        b"High\n",
        "the blob content must be the variant name plus the mandatory trailing newline"
    );
}

/// Flipping a unit-variant field between two variants changes that field's
/// blob content, not merely its tree-entry name — the property that keeps
/// `git diff` non-empty.
#[test]
fn unit_variant_field_change_changes_the_blob() {
    let (low_root, low_store) = serialize(&WithPriority {
        priority: Priority::Low,
    })
    .expect("serialize ok");
    let (high_root, high_store) = serialize(&WithPriority {
        priority: Priority::High,
    })
    .expect("serialize ok");
    assert_ne!(
        low_root, high_root,
        "changing the active unit variant must change the struct's root id"
    );
    let low_entry = find_entry(&low_store, &low_root, "priority");
    let high_entry = find_entry(&high_store, &high_root, "priority");
    assert_ne!(
        low_entry.oid, high_entry.oid,
        "the `priority` entry's own oid must differ, not just the root"
    );
    assert_eq!(
        low_store
            .get_blob(&low_entry.oid)
            .expect("read object")
            .expect("blob"),
        b"Low\n"
    );
    assert_eq!(
        high_store
            .get_blob(&high_entry.oid)
            .expect("read object")
            .expect("blob"),
        b"High\n"
    );
}

/// A tuple variant's sole entry is named after it.
#[test]
fn tuple_variant_is_named() {
    let (root_id, store) = serialize(&Shape::Pair(1, 2)).expect("serialize ok");
    let _ = find_entry(&store, &root_id, "Pair");
}

/// Struct-variant fields are encoded under the variant entry, named by field name.
#[test]
fn struct_variant_fields_named_by_field() {
    let (root_id, store) = serialize(&Shape::Circle { radius: 2.5 }).expect("serialize ok");
    let circle = find_entry(&store, &root_id, "Circle");
    assert_eq!(circle.mode.kind(), EntryKind::Tree);
    let fields = tree_entries(&store, &circle.oid);
    assert_eq!(fields.len(), 1);
    let radius = find_entry(&store, &circle.oid, "radius");
    assert_eq!(
        radius.mode.kind(),
        EntryKind::Blob,
        "`radius` must be a leaf blob"
    );
    assert_eq!(
        store.get_blob(&radius.oid).expect("read blob"),
        Some(b"2.5\n".to_vec())
    );
}

/// Multi-field tuple variants use width-three lowercase hexadecimal ordinals.
#[test]
fn tuple_variant_fields_named_by_index() {
    let (root_id, store) = serialize(&Shape::Pair(7, 13)).expect("serialize ok");
    let pair = find_entry(&store, &root_id, "Pair");
    assert_eq!(pair.mode.kind(), EntryKind::Tree);
    let fields = tree_entries(&store, &pair.oid);
    assert_eq!(fields.len(), 2);
    for (entry, (name, blob)) in fields
        .iter()
        .zip([("000", b"7\n".as_slice()), ("001", b"13\n".as_slice())])
    {
        assert_eq!(entry.filename, name);
        assert_eq!(entry.mode.kind(), EntryKind::Blob);
        assert_eq!(
            store.get_blob(&entry.oid).expect("read object"),
            Some(blob.to_vec())
        );
    }
}

/// A single tuple field is the tag's direct target, not an ordinal wrapper.
#[test]
fn single_tuple_variant_field_is_direct() {
    let text = Shape::Text("payload\n".into());
    let (root, store) = serialize(&text).expect("serialize text variant");
    let entries = tree_entries(&store, &root);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "Text");
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store.get_blob(&entries[0].oid).expect("read text"),
        Some(b"payload\n\n".to_vec())
    );
    assert_eq!(roundtrip(text), Shape::Text("payload\n".into()));

    let numbers = Shape::Numbers(vec![7, 13]);
    let (root, store) = serialize(&numbers).expect("serialize list variant");
    let entries = tree_entries(&store, &root);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "Numbers");
    assert_eq!(entries[0].mode.kind(), EntryKind::Tree);
    let elements = tree_entries(&store, &entries[0].oid);
    assert_eq!(elements.len(), 2);
    for (entry, (name, blob)) in elements
        .iter()
        .zip([("000", b"7\n".as_slice()), ("001", b"13\n".as_slice())])
    {
        assert_eq!(entry.filename, name);
        assert_eq!(entry.mode.kind(), EntryKind::Blob);
        assert_eq!(
            store.get_blob(&entry.oid).expect("read object"),
            Some(blob.to_vec())
        );
    }
    assert_eq!(roundtrip(numbers), Shape::Numbers(vec![7, 13]));

    let bytes = Shape::Bytes(vec![0, 0xff, b'\n']);
    let (root, store) = serialize(&bytes).expect("serialize bytes variant");
    let entries = tree_entries(&store, &root);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "Bytes");
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store.get_blob(&entries[0].oid).expect("read bytes"),
        Some(vec![0, 0xff, b'\n'])
    );
    assert_eq!(roundtrip(bytes), Shape::Bytes(vec![0, 0xff, b'\n']));
}

#[test]
fn variant_and_field_names_are_escaped() {
    let (root, store) = serialize(&Shape::Reserved).expect("serialize unit");
    assert_eq!(
        store.get_blob(&root).expect("read tag"),
        Some(b"%none\n".to_vec())
    );
    assert_eq!(roundtrip(Shape::Reserved), Shape::Reserved);

    let value = Shape::Renamed { value: 42 };
    let (root, store) = serialize(&value).expect("serialize struct variant");
    let entries = tree_entries(&store, &root);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "%a%2Fb");
    assert_eq!(entries[0].mode.kind(), EntryKind::Tree);
    let fields = tree_entries(&store, &entries[0].oid);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].filename, "%some");
    assert_eq!(fields[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store.get_blob(&fields[0].oid).expect("read field"),
        Some(b"42\n".to_vec())
    );
    assert_eq!(roundtrip(value), Shape::Renamed { value: 42 });
}

// --- equality ---

/// Distinct variants of the same enum produce distinct root object IDs.
#[test]
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
fn unit_variant_roundtrip() {
    assert_eq!(roundtrip(Shape::Unit), Shape::Unit);
}

#[test]
fn struct_variant_roundtrip() {
    assert_eq!(
        roundtrip(Shape::Circle { radius: 3.5 }),
        Shape::Circle { radius: 3.5 }
    );
}

#[test]
fn tuple_variant_roundtrip() {
    assert_eq!(roundtrip(Shape::Pair(-1, 99)), Shape::Pair(-1, 99));
}
