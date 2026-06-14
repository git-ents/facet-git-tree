//! Integration tests for serialization structure.
//!
//! Covers spec requirements:
//!   serialization.mechanism — a serialize function exists and accepts Facet types
//!   serialization.design.leaves — scalar fields serialize to UTF-8 blobs
//!   serialization.design.trees.schemas — schema objects carry a `.schema` sentinel tree
//!   serialization.design.trees.shape — `.schema` contains an `id` blob and a flat `defs` tree

use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{Nested, Person, Point, find_entry};

// --- serialization.mechanism ---

/// The `serialize` function is callable with any Facet type and returns a root ID
/// plus an object store containing at least one object.
#[test]
#[ignore = "serialization not yet implemented"]
fn serialize_returns_non_empty_store() {
    let (root_id, store) = serialize(&Point { x: 1.0, y: 2.0 }).expect("serialize should succeed");
    assert!(
        store.get(&root_id).is_some(),
        "root id must resolve in the store"
    );
}

// --- serialization.design.leaves ---

/// Scalar struct fields are stored as UTF-8 blobs, not sub-trees.
#[test]
#[ignore = "serialization not yet implemented"]
fn scalar_fields_are_blobs() {
    let (root_id, store) = serialize(&Point { x: 1.0, y: 2.0 }).expect("serialize should succeed");

    let x_entry = find_entry(&store, &root_id, "x");
    assert_eq!(
        x_entry.mode.kind(),
        EntryKind::Blob,
        "field `x` must be a blob"
    );

    let y_entry = find_entry(&store, &root_id, "y");
    assert_eq!(
        y_entry.mode.kind(),
        EntryKind::Blob,
        "field `y` must be a blob"
    );
}

/// Blob content for numeric fields is valid UTF-8.
#[test]
#[ignore = "serialization not yet implemented"]
fn blob_content_is_utf8() {
    let (root_id, store) = serialize(&Point { x: 2.5, y: -1.0 }).expect("serialize should succeed");

    for field in ["x", "y"] {
        let entry = find_entry(&store, &root_id, field);
        let bytes = store
            .get_blob(&entry.oid)
            .unwrap_or_else(|| panic!("blob missing for field {field}"));
        std::str::from_utf8(&bytes)
            .unwrap_or_else(|_| panic!("field {field} blob is not valid UTF-8"));
    }
}

/// Blob content for a string field is the UTF-8 encoding of the string value.
#[test]
#[ignore = "serialization not yet implemented"]
fn string_field_blob_content() {
    let (root_id, store) = serialize(&Person {
        name: "Alice".to_string(),
        age: 30,
        active: true,
    })
    .expect("serialize should succeed");

    let entry = find_entry(&store, &root_id, "name");
    let bytes = store
        .get_blob(&entry.oid)
        .expect("name blob must be present");
    assert_eq!(
        bytes, b"Alice",
        "string field should encode as its UTF-8 bytes"
    );
}

// --- serialization.design.trees.schemas ---

/// Every serialized schema object has a `.schema` sentinel entry whose mode is Tree.
#[test]
#[ignore = "serialization not yet implemented"]
fn schema_sentinel_is_present_and_is_tree() {
    let (root_id, store) = serialize(&Point { x: 0.0, y: 0.0 }).expect("serialize should succeed");

    let schema_entry = find_entry(&store, &root_id, ".schema");
    assert_eq!(
        schema_entry.mode.kind(),
        EntryKind::Tree,
        "`.schema` sentinel must be a tree"
    );
}

/// Nested schema objects each carry their own `.schema` tree.
#[test]
#[ignore = "serialization not yet implemented"]
fn nested_struct_has_schema_sentinel() {
    let (root_id, store) = serialize(&Nested {
        location: Point { x: 1.0, y: 2.0 },
        label: "origin".to_string(),
    })
    .expect("serialize should succeed");

    // The nested `location` field should itself be a tree (schema object).
    let location_entry = find_entry(&store, &root_id, "location");
    assert_eq!(
        location_entry.mode.kind(),
        EntryKind::Tree,
        "nested struct field must be encoded as a tree"
    );

    // That sub-tree must also have a `.schema` sentinel.
    let _inner_schema = find_entry(&store, &location_entry.oid, ".schema");
}

// --- serialization.design.trees.shape ---

/// The `.schema` tree contains an `id` entry that resolves to a blob.
#[test]
#[ignore = "serialization not yet implemented"]
fn schema_contains_id_blob() {
    let (root_id, store) = serialize(&Point { x: 0.0, y: 0.0 }).expect("serialize should succeed");

    let schema_entry = find_entry(&store, &root_id, ".schema");
    let id_entry = find_entry(&store, &schema_entry.oid, "id");
    assert_eq!(
        id_entry.mode.kind(),
        EntryKind::Blob,
        "`.schema/id` must be a blob"
    );

    // The type identifier must be non-empty UTF-8.
    let id_bytes = store
        .get_blob(&id_entry.oid)
        .expect("`.schema/id` blob must be in store");
    assert!(!id_bytes.is_empty(), "type identifier must be non-empty");
    std::str::from_utf8(&id_bytes).expect("type identifier must be valid UTF-8");
}

/// The `.schema/defs` subtree is flat — it contains only blob entries, never nested trees.
///
/// This flatness is required by the spec to prevent cycles in type definitions.
#[test]
#[ignore = "serialization not yet implemented"]
fn schema_defs_is_flat() {
    let (root_id, store) = serialize(&Point { x: 0.0, y: 0.0 }).expect("serialize should succeed");

    assert_defs_flat(&store, &root_id);
}

/// `defs` flatness must hold even for a type that *references another named type*:
/// `Nested` embeds a `Point`, and the spec requires the inner type to be recorded
/// by identifier in a flat `defs`, never inlined as a full schema tree. With `Point`
/// inlined, `defs` would contain a nested tree and this assertion would fail.
#[test]
#[ignore = "serialization not yet implemented"]
fn nested_type_schema_defs_is_flat() {
    let (root_id, store) = serialize(&Nested {
        location: Point { x: 1.0, y: 2.0 },
        label: "origin".to_string(),
    })
    .expect("serialize should succeed");

    assert_defs_flat(&store, &root_id);
}

/// Two distinct schema objects for the same type have the same `id` blob content.
#[test]
#[ignore = "serialization not yet implemented"]
fn same_type_has_same_schema_id() {
    let (root1, store1) = serialize(&Point { x: 1.0, y: 2.0 }).expect("serialize should succeed");
    let (root2, store2) = serialize(&Point { x: 9.0, y: -3.0 }).expect("serialize should succeed");

    assert_eq!(
        schema_id_bytes(&store1, &root1),
        schema_id_bytes(&store2, &root2),
        "same Facet type must have identical schema id regardless of value"
    );
}

/// Distinct types have distinct `.schema/id` content, so a schema id actually
/// identifies the type (and never collides across types).
#[test]
#[ignore = "serialization not yet implemented"]
fn different_types_have_different_schema_ids() {
    let (point_root, point_store) =
        serialize(&Point { x: 0.0, y: 0.0 }).expect("serialize should succeed");
    let (person_root, person_store) = serialize(&Person {
        name: "Alice".to_string(),
        age: 1,
        active: true,
    })
    .expect("serialize should succeed");

    assert_ne!(
        schema_id_bytes(&point_store, &point_root),
        schema_id_bytes(&person_store, &person_root),
        "different Facet types must have different schema ids"
    );
}

// --- helpers local to schema-shape assertions ---

/// The bytes of a root object's `.schema/id` blob.
fn schema_id_bytes(
    store: &facet_git_tree::ObjectStore,
    root: &facet_git_tree::ObjectId,
) -> Vec<u8> {
    let schema = find_entry(store, root, ".schema");
    let id = find_entry(store, &schema.oid, "id");
    store.get_blob(&id.oid).expect("`.schema/id` blob in store")
}

/// Assert a root object's `.schema/defs` is a tree of only blobs (no nested trees).
fn assert_defs_flat(store: &facet_git_tree::ObjectStore, root: &facet_git_tree::ObjectId) {
    let schema_entry = find_entry(store, root, ".schema");
    let defs_entry = find_entry(store, &schema_entry.oid, "defs");
    assert_eq!(
        defs_entry.mode.kind(),
        EntryKind::Tree,
        "`.schema/defs` must be a tree"
    );

    let defs_entries = store
        .get_tree(&defs_entry.oid)
        .expect("`.schema/defs` must be in store");
    for entry in defs_entries {
        assert_eq!(
            entry.mode.kind(),
            EntryKind::Blob,
            "`.schema/defs` entry `{}` must be a blob (flat — no nested trees)",
            entry.filename
        );
    }
}
