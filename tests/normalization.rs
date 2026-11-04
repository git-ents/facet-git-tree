//! Integration tests for leaf value normalization on write.
//!
//! The frozen format requires both signs of zero to be written as `0.0\n`
//! and every NaN payload to be written as `nan\n`.

use facet::Facet;
use facet_git_tree::{EntryKind, deserialize, serialize};

#[derive(Debug, Facet)]
struct WithF32 {
    v: f32,
}

#[derive(Debug, Facet)]
struct WithF64 {
    v: f64,
}

/// Retrieve the blob bytes for the single-field wrapper struct's `v` entry.
fn serialize_scalar_f64(value: f64) -> Vec<u8> {
    let (root_id, store) = serialize(&WithF64 { v: value }).expect("serialize should succeed");
    let entries = store
        .get_tree(&root_id)
        .expect("read object")
        .expect("root must be a tree");
    let v_entry = entries
        .iter()
        .find(|e| e.filename == "v")
        .expect("struct must have field `v`");
    assert_eq!(
        v_entry.mode.kind(),
        EntryKind::Blob,
        "scalar `v` must serialize to a blob"
    );
    store
        .get_blob(&v_entry.oid)
        .expect("read object")
        .expect("blob must be in store")
        .to_vec()
}

fn serialize_scalar_f32(value: f32) -> Vec<u8> {
    let (root_id, store) = serialize(&WithF32 { v: value }).expect("serialize should succeed");
    let entries = store
        .get_tree(&root_id)
        .expect("read object")
        .expect("root must be a tree");
    let v_entry = entries
        .iter()
        .find(|e| e.filename == "v")
        .expect("struct must have field `v`");
    store
        .get_blob(&v_entry.oid)
        .expect("read object")
        .expect("blob must be in store")
        .to_vec()
}

// --- negative zero normalization ---

/// f64 negative zero and positive zero produce identical blob content.
#[test]

fn f64_negative_zero_normalized_to_positive_zero() {
    let pos_zero = serialize_scalar_f64(0.0_f64);
    let neg_zero = serialize_scalar_f64(-0.0_f64);
    assert_eq!(
        pos_zero, neg_zero,
        "f64 -0.0 must serialize identically to +0.0"
    );
    assert_eq!(pos_zero, b"0.0\n");
    let (id, store) = serialize(&WithF64 { v: -0.0 }).unwrap();
    assert_eq!(deserialize::<WithF64>(&id, &store).unwrap().v.to_bits(), 0);
}

/// f32 negative zero and positive zero produce identical blob content.
#[test]

fn f32_negative_zero_normalized_to_positive_zero() {
    let pos_zero = serialize_scalar_f32(0.0_f32);
    let neg_zero = serialize_scalar_f32(-0.0_f32);
    assert_eq!(
        pos_zero, neg_zero,
        "f32 -0.0 must serialize identically to +0.0"
    );
    assert_eq!(pos_zero, b"0.0\n");
    let (id, store) = serialize(&WithF32 { v: -0.0 }).unwrap();
    assert_eq!(deserialize::<WithF32>(&id, &store).unwrap().v.to_bits(), 0);
}

/// Two structs that differ only in the sign of zero produce the same root object ID.
///
/// This verifies that normalization happens before hashing, ensuring that
/// positive and negative zero are structurally equal.
#[test]

fn negative_zero_structural_equality() {
    let (id_pos, _) = serialize(&WithF64 { v: 0.0_f64 }).expect("serialize should succeed");
    let (id_neg, _) = serialize(&WithF64 { v: -0.0_f64 }).expect("serialize should succeed");
    assert_eq!(
        id_pos, id_neg,
        "structs with +0.0 and -0.0 must have identical root object IDs"
    );
}

// --- NaN normalization ---

/// f64 NaN serializes to the unquoted string literal "nan".
#[test]

fn f64_nan_normalized_to_string_nan() {
    let bytes = serialize_scalar_f64(f64::NAN);
    assert_eq!(
        bytes, b"nan\n",
        "f64 NaN must serialize to the unquoted string `nan`"
    );
    let (id, store) = serialize(&WithF64 { v: f64::NAN }).unwrap();
    assert_eq!(
        deserialize::<WithF64>(&id, &store).unwrap().v.to_bits(),
        0x7ff8_0000_0000_0000
    );
}

/// f32 NaN serializes to the unquoted string literal "nan".
#[test]

fn f32_nan_normalized_to_string_nan() {
    let bytes = serialize_scalar_f32(f32::NAN);
    assert_eq!(
        bytes, b"nan\n",
        "f32 NaN must serialize to the unquoted string `nan`"
    );
    let (id, store) = serialize(&WithF32 { v: f32::NAN }).unwrap();
    assert_eq!(
        deserialize::<WithF32>(&id, &store).unwrap().v.to_bits(),
        0x7fc0_0000
    );
}

/// Quiet and signaling NaNs of either sign share the canonical NaN blob.
#[test]

fn different_nan_payloads_normalize_identically() {
    for bits in [
        0x7ff8_0000_0000_0001,
        0x7ff0_0000_0000_0001,
        0xfff8_0000_0000_1234,
    ] {
        let value = f64::from_bits(bits);
        assert!(value.is_nan());
        assert_eq!(serialize_scalar_f64(value), b"nan\n", "f64 {bits:#x}");
    }
    for bits in [0x7fc0_0001, 0x7f80_0001, 0xffc0_1234] {
        let value = f32::from_bits(bits);
        assert!(value.is_nan());
        assert_eq!(serialize_scalar_f32(value), b"nan\n", "f32 {bits:#x}");
    }
}
