//! Integration tests for the dynamic value deserialization heuristic.
//!
//! Covers spec requirement:
//!   deserialization.dynamic.heuristic
//!     — with no type marker on disk, a dynamic read recovers shape from the
//!       object graph: a UTF-8 text leaf is a String, a non-UTF-8 blob is
//!       Bytes, a contiguous ordinal tree is an Array, the `none` marker is
//!       Null, and other named trees are Objects. Kinds with the same encoding
//!       collide.

use std::collections::HashMap;

use facet_git_tree::{
    DeserializeError, EntryKind, EntryMode, ObjectStore, TreeEntry, deserialize, serialize,
};
use facet_value::{Value, value};
use gix_object::{Kind, Write as _};

mod common;
use common::{Person, WithMap};

// --- blobs ---

/// A text leaf whose contents are valid UTF-8 reads back as a String.
#[test]
fn utf8_blob_reads_as_string() -> anyhow::Result<()> {
    let (root, store) = serialize(&"hello".to_string())?;
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, Value::from("hello"));
    Ok(())
}

/// A dynamic `Bytes` value, whose text leaf is not valid UTF-8, reads back as
/// the same `Bytes`.
///
/// A dynamic `Bytes` is written as a text leaf (raw bytes plus one trailing
/// `\n`), unlike a typed `Vec<u8>` `bytes` leaf, so the fixture must go through
/// the dynamic writer for the heuristic to see a valid encoding.
#[test]
fn non_utf8_blob_reads_as_bytes() -> anyhow::Result<()> {
    let data: Vec<u8> = vec![0xff, 0xfe, 0x00];
    let dynamic = Value::from(data.clone());
    let (root, store) = serialize(&dynamic)?;
    assert_eq!(
        store.get_blob(&root)?.as_deref(),
        Some([data.as_slice(), b"\n"].concat().as_slice())
    );
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, Value::from(data));
    Ok(())
}

// --- trees ---

/// A non-empty tree with contiguous three-digit lowercase hexadecimal
/// ordinals reads back as an Array, in ordinal order.
#[test]
fn ordinal_tree_reads_as_array() -> anyhow::Result<()> {
    let items = vec!["x".to_string(), "y".to_string()];
    let (root, store) = serialize(&items)?;
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, value!(["x", "y"]));
    Ok(())
}

/// A tree with non-ordinal names reads back as an Object.
#[test]
fn named_tree_reads_as_object() -> anyhow::Result<()> {
    let (root, store) = serialize(&common::Config {
        name: "n".into(),
        value: 5,
    })?;
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, value!({ "name": "n", "value": "5" }));
    Ok(())
}

/// The `none` marker reads back as Null; it also represents empty containers
/// and unit.
#[test]
fn marker_tree_reads_as_null() -> anyhow::Result<()> {
    let (root, store) = serialize(&Value::NULL)?;
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, Value::NULL);
    Ok(())
}

/// A tree mixing an ordinal name with a non-ordinal one is not all-ordinal, so
/// it reads back as an Object keyed by the literal names.
#[test]
fn mixed_ordinal_and_named_tree_reads_as_object() -> anyhow::Result<()> {
    let mut table = HashMap::new();
    table.insert("000".to_string(), "a".to_string());
    table.insert("name".to_string(), "b".to_string());
    let (root, store) = serialize(&WithMap { table })?;
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, value!({ "table": { "000": "a", "name": "b" } }));
    Ok(())
}

/// Noncontiguous ordinal-looking names can be object keys, so they must not
/// be decoded as an array with missing elements.
#[test]
fn noncontiguous_ordinal_names_read_as_object() -> anyhow::Result<()> {
    let original = value!({ "000": "x", "002": "y" });
    let (root, store) = serialize(&original)?;
    let decoded: Value = deserialize(&root, &store)?;
    assert_eq!(decoded, original);
    assert_eq!(serialize(&decoded)?.0, root);
    Ok(())
}

/// Escaped object keys are decoded back to their original user names.
#[test]
fn escaped_names_read_as_object_keys() -> anyhow::Result<()> {
    let original = value!({
        "bad/key": "slash",
        "none": "reserved",
        "some": "present",
        ".schema": "dot",
        "": "empty",
        "naïve": "unicode"
    });
    let (root, store) = serialize(&original)?;
    let decoded: Value = deserialize(&root, &store)?;
    assert_eq!(decoded, original);
    assert_eq!(serialize(&decoded)?.0, root);
    Ok(())
}

// --- typed values under the heuristic ---

/// A typed struct reads back as an Object of Strings: field names survive, but
/// scalar leaves (numbers, bools) have no marker and come back textual.
#[test]
fn typed_person_reads_as_object_of_strings() -> anyhow::Result<()> {
    let (root, store) = serialize(&Person {
        name: "Ada".into(),
        age: 36,
        active: true,
    })?;
    let v: Value = deserialize(&root, &store)?;
    assert_eq!(v, value!({ "name": "Ada", "age": "36", "active": "true" }));
    Ok(())
}

// --- recursion guard ---

/// Heuristic recursion is bounded by the same depth guard as typed
/// recursion: a tree nested deeper fails rather than overflowing the stack.
#[test]
fn depth_beyond_max_is_rejected() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let mut root = store.write_buf(Kind::Blob, b"leaf\n").expect("write leaf");
    for _ in 0..40 {
        root = store
            .write(&gix_object::Tree {
                entries: vec![TreeEntry {
                    mode: EntryMode::from(EntryKind::Tree),
                    filename: "000".into(),
                    oid: root,
                }],
            })
            .expect("write nesting tree");
    }
    let err = deserialize::<Value>(&root, &store).unwrap_err();
    assert!(matches!(err, DeserializeError::MaxDepth(_)), "got {err:?}");
    Ok(())
}
