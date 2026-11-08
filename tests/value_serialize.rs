//! Integration tests for dynamic value serialization.
//!
//! Covers spec requirement:
//!   serialization.design.dynamic
//!     — a dynamic value serializes as its runtime kind's typed encoding:
//!       scalar kinds as text leaves, arrays as ordinal-keyed trees, objects
//!       as sorted trees with escaped names, and null as the `none` marker.

use facet_git_tree::{EntryKind, serialize};
use facet_value::{VArray, VDateTime, VObject, Value, value};

mod common;
use common::{HELLO_LEAF_BLOB_OID, WithValue, find_entry, tree_entries, typed};

// --- scalar kinds ---

/// A dynamic string is its UTF-8 bytes verbatim — the same blob (and thus the
/// same OID) as the equivalent typed `String`.
#[test]
fn string_blob_matches_typed_string() -> anyhow::Result<()> {
    let (dyn_root, dyn_store) = serialize(&Value::from("hello"))?;
    let (typed_root, _) = serialize(&"hello".to_string())?;
    assert_eq!(dyn_root, typed_root);
    assert_eq!(dyn_root.as_bytes(), HELLO_LEAF_BLOB_OID.as_slice());
    assert_eq!(
        dyn_store
            .get_blob(&dyn_root)
            .expect("read object")
            .expect("blob"),
        b"hello\n"
    );
    Ok(())
}

/// Dynamic bytes are a text leaf: their bytes are followed by one newline.
#[test]
fn bytes_are_text_leaves() -> anyhow::Result<()> {
    for bytes in [vec![], vec![0xff, 0, b'\n'], b"hello".to_vec()] {
        let (root, store) = serialize(&Value::from(bytes.clone()))?;
        let mut expected = bytes;
        expected.push(b'\n');
        assert_eq!(store.get_blob(&root)?.as_deref(), Some(expected.as_slice()));
    }
    Ok(())
}

/// A dynamic bool is its textual form, exactly as a typed `bool` would encode.
#[test]
fn bool_is_textual_blob() -> anyhow::Result<()> {
    let (root, store) = serialize(&Value::from(true))?;
    assert_eq!(
        store.get_blob(&root).expect("read object").expect("blob"),
        b"true\n"
    );
    Ok(())
}

/// Dynamic integers are their decimal text, signed and unsigned alike —
/// including a u64 too large for i64, which exercises the unsigned read.
#[test]
fn numbers_are_decimal_blobs() -> anyhow::Result<()> {
    let (root, store) = serialize(&Value::from(42i64))?;
    assert_eq!(
        store.get_blob(&root).expect("read object").expect("blob"),
        b"42\n"
    );

    let (root, store) = serialize(&Value::from(-7i64))?;
    assert_eq!(
        store.get_blob(&root).expect("read object").expect("blob"),
        b"-7\n"
    );

    let (root, store) = serialize(&Value::from(u64::MAX))?;
    assert_eq!(
        store.get_blob(&root).expect("read object").expect("blob"),
        b"18446744073709551615\n"
    );
    Ok(())
}

/// Null is the `none` marker tree: a single entry pointing to an empty blob,
/// not a literal empty tree that would be invisible to `git ls-tree -r`/`diff`.
/// Empty binary leaves share its blob but not its tree.
#[test]
fn null_is_marker_tree() -> anyhow::Result<()> {
    let (root, store) = serialize(&Value::NULL)?;
    let entries = tree_entries(&store, &root);
    assert_eq!(
        entries.len(),
        1,
        "Null must be the single-entry marker tree"
    );
    assert_eq!(entries[0].filename, "none");
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store
            .get_blob(&typed(&entries[0].oid))
            .expect("read object")
            .expect("marker blob"),
        b"",
        "the marker blob must be empty"
    );
    Ok(())
}

// --- containers ---

/// A dynamic array is an ordinal-keyed tree, exactly like a typed `Vec`.
#[test]
fn array_is_ordinal_tree() -> anyhow::Result<()> {
    let (root, store) = serialize(&value!(["a", "b", "c"]))?;
    let names: Vec<_> = tree_entries(&store, &root)
        .into_iter()
        .map(|e| e.filename.to_string())
        .collect();
    assert_eq!(names, ["000", "001", "002"]);
    let (typed_root, _) = serialize(&vec!["a".to_string(), "b".to_string(), "c".to_string()])?;
    assert_eq!(root, typed_root);
    let first = find_entry(&store, &root, "000");
    assert_eq!(
        store
            .get_blob(&typed(&first.oid))
            .expect("read object")
            .expect("blob"),
        b"a\n"
    );
    Ok(())
}

/// Ordinals remain hexadecimal beyond the decimal digits.
#[test]
fn array_ordinals_are_lowercase_hex() -> anyhow::Result<()> {
    let mut array = VArray::new();
    for i in 0..17 {
        array.push(i);
    }
    let (root, store) = serialize(&Value::from(array))?;
    let entries = tree_entries(&store, &root);
    assert_eq!(entries.len(), 17);
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(entry.filename.to_string(), format!("{i:03x}"));
        assert_eq!(entry.mode.kind(), EntryKind::Blob);
        assert_eq!(
            store.get_blob(&typed(&entry.oid))?.as_deref(),
            Some(format!("{i}\n").as_bytes())
        );
    }
    Ok(())
}

/// A dynamic object is a name-keyed tree with entries in sorted order,
/// independent of insertion order.
#[test]
fn object_is_sorted_name_keyed_tree() -> anyhow::Result<()> {
    let mut obj = VObject::new();
    obj.insert("zeta", "z");
    obj.insert("alpha", "a");
    let (root, store) = serialize(&Value::from(obj))?;
    let names: Vec<_> = tree_entries(&store, &root)
        .into_iter()
        .map(|e| e.filename.to_string())
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
    let alpha = find_entry(&store, &root, "alpha");
    assert_eq!(
        store
            .get_blob(&typed(&alpha.oid))
            .expect("read object")
            .expect("blob"),
        b"a\n"
    );
    Ok(())
}

/// Null, an empty array, an empty object, unit, and a typed empty sequence
/// share the same marker tree.
#[test]
fn empty_object_is_marker_tree() -> anyhow::Result<()> {
    let (root, store) = serialize(&Value::from(VObject::new()))?;
    let entries = tree_entries(&store, &root);
    assert_eq!(entries.len(), 1, "empty Object must be the marker tree");
    assert_eq!(entries[0].filename, "none");
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(
        store.get_blob(&typed(&entries[0].oid))?.as_deref(),
        Some(b"".as_slice())
    );

    let (null_root, _) = serialize(&Value::NULL)?;
    let (array_root, _) = serialize(&Value::from(VArray::new()))?;
    let (unit_root, _) = serialize(&())?;
    let (sequence_root, _) = serialize(&Vec::<String>::new())?;
    assert_eq!(root, null_root);
    assert_eq!(root, array_root);
    assert_eq!(root, unit_root);
    assert_eq!(root, sequence_root);

    // An empty binary leaf shares the marker target, but is a blob, not a tree.
    let (empty_bytes, _) = serialize(&Vec::<u8>::new())?;
    assert_eq!(entries[0].oid, empty_bytes);
    assert_ne!(root, empty_bytes);
    Ok(())
}

// --- key validation ---

/// Arbitrary object keys use canonical escaped names, including a slash.
#[test]
fn object_keys_are_escaped() -> anyhow::Result<()> {
    let mut obj = VObject::new();
    for (key, value) in [
        ("bad/key", "slash"),
        ("none", "reserved"),
        ("some", "present"),
        (".schema", "dot"),
        ("", "empty"),
        ("naïve", "unicode"),
    ] {
        obj.insert(key, value);
    }
    let (root, store) = serialize(&Value::from(obj))?;
    for (encoded, value) in [
        ("%bad%2Fkey", "slash\n"),
        ("%none", "reserved\n"),
        ("%some", "present\n"),
        ("%.schema", "dot\n"),
        ("%", "empty\n"),
        ("%na%C3%AFve", "unicode\n"),
    ] {
        let entry = find_entry(&store, &root, encoded);
        assert_eq!(entry.mode.kind(), EntryKind::Blob, "{encoded}");
        assert_eq!(
            store.get_blob(&typed(&entry.oid))?.as_deref(),
            Some(value.as_bytes()),
            "{encoded}"
        );
    }
    assert_eq!(tree_entries(&store, &root).len(), 6);
    Ok(())
}

/// The length limit applies to the encoded name, not just the original key.
#[test]
fn object_key_over_encoded_length_limit_is_rejected() {
    for key in ["a".repeat(256), format!(".{}", "a".repeat(254))] {
        let mut obj = VObject::new();
        obj.insert(key.as_str(), "x");
        assert!(serialize(&Value::from(obj)).is_err(), "key {key:?}");
    }
}

// --- composition with typed values ---

/// A `Value` field inside a typed struct encodes exactly as the standalone
/// dynamic value would: the field entry's OID equals the standalone root.
#[test]
fn value_nested_in_typed_struct() -> anyhow::Result<()> {
    let meta = value!({ "k": "v" });
    let (root, store) = serialize(&WithValue { meta: meta.clone() })?;
    let entry = find_entry(&store, &root, "meta");
    let (standalone_root, _) = serialize(&meta)?;
    assert_eq!(entry.oid, standalone_root);
    let k = find_entry(&store, &entry.oid, "k");
    assert_eq!(
        store
            .get_blob(&typed(&k.oid))
            .expect("read object")
            .expect("blob"),
        b"v\n"
    );
    Ok(())
}

// --- floats beyond 64/128 bits ---

/// A finite float whose magnitude exceeds both `i128::MAX` and `u128::MAX`
/// still serializes: the `facet_value::Value` downcast — unconditional, so
/// this holds with and without the `value` feature — can tell, via
/// `VNumber::to_i128`/`to_u128` both returning `None`, that the value is
/// genuinely float-backed rather than the lossy image of an out-of-range
/// integer, so it renders through the same float encoding a typed `f64`
/// would use, producing an identical blob and OID.
#[test]
fn large_finite_float_matches_typed_f64() -> anyhow::Result<()> {
    let (dyn_root, dyn_store) = serialize(&Value::from(1e40_f64))?;
    let (typed_root, typed_store) = serialize(&1e40_f64)?;
    assert_eq!(dyn_root, typed_root);
    assert_eq!(
        dyn_store
            .get_blob(&dyn_root)
            .expect("read object")
            .expect("blob"),
        typed_store
            .get_blob(&typed_root)
            .expect("read object")
            .expect("blob"),
    );
    Ok(())
}

/// A whole-valued float that fits `u128` (unlike `1e40` above) exercises the
/// same downcast through its other branch — `to_u128` succeeds — and must
/// still match the typed encoding exactly.
#[test]
fn avogadro_float_matches_typed_f64() -> anyhow::Result<()> {
    let (dyn_root, dyn_store) = serialize(&Value::from(6.022e23_f64))?;
    let (typed_root, typed_store) = serialize(&6.022e23_f64)?;
    assert_eq!(dyn_root, typed_root);
    assert_eq!(
        dyn_store
            .get_blob(&dyn_root)
            .expect("read object")
            .expect("blob"),
        typed_store
            .get_blob(&typed_root)
            .expect("read object")
            .expect("blob"),
    );
    Ok(())
}

// --- datetime ---

/// A negative (BCE) year renders as `-` followed by its magnitude
/// zero-padded to four digits. Zero-padding the whole signed value instead
/// (`format!("{year:04}", ...)` with `year: i32`) would count the sign
/// character toward the width and under-pad the magnitude by one digit.
#[test]
fn datetime_negative_year_is_zero_padded_by_magnitude() -> anyhow::Result<()> {
    let dt = VDateTime::new_local_date(-5, 6, 15);
    let (root, store) = serialize(&Value::from(dt))?;
    assert_eq!(
        store.get_blob(&root).expect("read object").expect("blob"),
        b"-0005-06-15\n"
    );
    Ok(())
}

// --- qname (`value` feature) ---

/// A `QName` with an empty-string namespace is indistinguishable from one
/// with no namespace at all: Clark notation reserves the empty-braces form
/// for "no namespace", so both constructions MUST serialize to the same
/// blob (and therefore the same OID).
#[cfg(feature = "value")]
#[test]
fn qname_empty_namespace_matches_no_namespace() -> anyhow::Result<()> {
    let with_empty_ns = facet_value::VQName::new("", "local");
    let no_ns = facet_value::VQName::new_local("local");
    let (root_a, _) = serialize(&Value::from(with_empty_ns))?;
    let (root_b, _) = serialize(&Value::from(no_ns))?;
    assert_eq!(root_a, root_b);
    Ok(())
}
