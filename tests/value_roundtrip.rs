//! Integration tests for dynamic value roundtrip behavior.
//!
//! Covers spec requirements:
//!   serialization.design.dynamic
//!     — the dynamic write mapping (blobs, ordinal trees, name-keyed trees).
//!   deserialization.dynamic.heuristic
//!     — untagged dynamic reads are lossy: different kinds can share the
//!       same encoding, while an unambiguous subset survives the heuristic.

use facet_git_tree::{deserialize, serialize};
use facet_value::{VArray, VObject, Value, value};
use proptest::prelude::*;

mod common;
use common::roundtrip;

// --- property: the faithful subset always roundtrips ---

/// A strategy over the faithful subset: strings, guaranteed-non-UTF-8 bytes,
/// non-empty arrays, and objects with non-ordinal keys, nested a few levels.
fn faithful_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        // Short alphabetic strings, including the empty string.
        "[a-z]{0,8}".prop_map(Value::from),
        // A leading 0xff byte is invalid anywhere in UTF-8, so these bytes can
        // never be mistaken for a String.
        proptest::collection::vec(any::<u8>(), 0..8).prop_map(|mut b| {
            b.insert(0, 0xff);
            Value::from(b)
        }),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            // Empty arrays share the marker with Null, so use non-empty arrays.
            proptest::collection::vec(inner.clone(), 1..4).prop_map(|items| {
                let mut arr = VArray::new();
                for item in items {
                    arr.push(item);
                }
                Value::from(arr)
            }),
            // Alphabetic keys cannot form an ordinal sequence starting at 000.
            // Empty objects share Null's marker and are tested separately.
            proptest::collection::btree_map("[a-z]{1,6}", inner, 1..4).prop_map(|entries| {
                let mut obj = VObject::new();
                for (k, v) in entries {
                    obj.insert(k.as_str(), v);
                }
                Value::from(obj)
            }),
        ]
    })
}

proptest! {
    // Do not mutate the existing regression file, whose saved empty-object
    // case belongs to the old heuristic and is covered explicitly below.
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    /// Every value in the faithful subset survives serialize → deserialize
    /// unchanged.
    #[test]
    fn faithful_subset_roundtrips(v in faithful_value()) {
        let (root, store) = serialize(&v).expect("serialize should succeed");
        let back: Value = deserialize(&root, &store).expect("deserialize should succeed");
        prop_assert_eq!(back, v);
    }
}

// --- the documented lossy mappings ---

/// String and number kinds, and numeric signedness and width, have no tags.
#[test]
fn numeric_type_and_width_degrade_to_string() -> anyhow::Result<()> {
    let string = Value::from("42");
    let signed = Value::from(42i64);
    let unsigned = Value::from(42u64);
    let (string_root, store) = serialize(&string)?;
    assert_eq!(
        store.get_blob(&string_root)?.as_deref(),
        Some(b"42\n".as_slice())
    );
    assert_eq!(serialize(&signed)?.0, string_root);
    assert_eq!(serialize(&unsigned)?.0, string_root);
    assert_eq!(roundtrip(string.clone()), string);
    assert_eq!(roundtrip(signed), string);
    assert_eq!(roundtrip(unsigned), string);
    Ok(())
}

/// Null and empty dynamic containers share the marker tree and read as
/// Null; the heuristic cannot distinguish them.
#[test]
fn null_and_empty_containers_degrade_to_null() {
    let empty_array = Value::from(VArray::new());
    let empty_object = Value::from(VObject::new());
    let expected = Value::NULL;

    assert_eq!(roundtrip(Value::NULL), expected);
    assert_eq!(roundtrip(empty_array), expected);
    assert_eq!(roundtrip(empty_object), expected);
}

/// An object with a contiguous ordinal sequence is interpreted as an Array,
/// but its original encoding remains unchanged.
#[test]
fn ordinal_keyed_object_degrades_to_array() -> anyhow::Result<()> {
    let object = value!({ "000": "x", "001": "y" });
    let array = value!(["x", "y"]);
    let (object_root, _) = serialize(&object)?;
    assert_eq!(serialize(&array)?.0, object_root);
    assert_eq!(roundtrip(object), array);
    Ok(())
}

/// Dynamic Bytes and String values with the same UTF-8 content have the same
/// text leaf and are read back as String.
#[test]
fn utf8_bytes_degrade_to_string() -> anyhow::Result<()> {
    let bytes = Value::from(b"hello".to_vec());
    let text = Value::from("hello");
    assert_eq!(serialize(&bytes)?.0, serialize(&text)?.0);
    assert_eq!(roundtrip(bytes), text);
    Ok(())
}

// --- content addressing ---

/// Equal dynamic values produce equal root OIDs, independent of construction
/// (object insertion) order — the content-addressing property.
#[test]
fn equal_values_share_a_root_oid() -> anyhow::Result<()> {
    let a = value!({ "k": ["x", "y"], "b": "c" });
    let mut obj = VObject::new();
    obj.insert("b", "c");
    obj.insert("k", value!(["x", "y"]));
    let b = Value::from(obj);
    assert_eq!(a, b);

    let (root_a, _) = serialize(&a)?;
    let (root_b, _) = serialize(&b)?;
    assert_eq!(root_a, root_b);

    // And serializing the same value twice is trivially stable.
    let (root_a2, _) = serialize(&a)?;
    assert_eq!(root_a, root_a2);
    Ok(())
}
