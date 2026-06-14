//! Integration tests for leaf byte encoding.
//!
//! Covers spec requirement:
//!   serialization.design.leaves.encoding
//!     — a leaf blob is the value's raw textual representation, with no delimiters,
//!       quoting, or trailing newline; a String is its UTF-8 bytes verbatim.

use facet::Facet;
use facet_git_tree::serialize;

mod common;
use common::find_entry;

// --- single-field scalar wrappers ---

#[derive(Debug, Facet)]
struct WithU32 {
    v: u32,
}

#[derive(Debug, Facet)]
struct WithI64 {
    v: i64,
}

#[derive(Debug, Facet)]
struct WithBool {
    v: bool,
}

#[derive(Debug, Facet)]
struct WithChar {
    v: char,
}

#[derive(Debug, Facet)]
struct WithString {
    v: String,
}

/// Serialize a single-field wrapper and return the raw bytes of its `v` leaf blob.
fn v_blob<T: for<'a> Facet<'a>>(value: &T) -> Vec<u8> {
    let (root_id, store) = serialize(value).expect("serialize should succeed");
    let entry = find_entry(&store, &root_id, "v");
    store
        .get_blob(&entry.oid)
        .expect("`v` must be a blob in store")
}

// --- integers ---

/// An unsigned integer encodes as its decimal text with no padding or sign.
#[test]
#[ignore = "serialization not yet implemented"]
fn unsigned_integer_textual_form() {
    assert_eq!(v_blob(&WithU32 { v: 42 }), b"42");
}

/// A signed integer encodes as its decimal text including the minus sign.
#[test]
#[ignore = "serialization not yet implemented"]
fn signed_integer_textual_form() {
    assert_eq!(v_blob(&WithI64 { v: -7 }), b"-7");
}

/// The extreme `i64::MIN` encodes exactly, with no overflow in formatting.
#[test]
#[ignore = "serialization not yet implemented"]
fn i64_min_textual_form() {
    assert_eq!(v_blob(&WithI64 { v: i64::MIN }), b"-9223372036854775808");
}

// --- bool ---
//
// The spec says a scalar is stored as "the bytes of its textual form" but does
// not name bool's textual form. These tests pin it to Rust's `Display`
// (`true`/`false`) rather than `1`/`0`; change them if the intended form differs.

/// `true` encodes as the literal text `true`.
#[test]
#[ignore = "serialization not yet implemented"]
fn bool_true_textual_form() {
    assert_eq!(v_blob(&WithBool { v: true }), b"true");
}

/// `false` encodes as the literal text `false`.
#[test]
#[ignore = "serialization not yet implemented"]
fn bool_false_textual_form() {
    assert_eq!(v_blob(&WithBool { v: false }), b"false");
}

// --- char ---

/// An ASCII char encodes as its single byte.
#[test]
#[ignore = "serialization not yet implemented"]
fn char_ascii_textual_form() {
    assert_eq!(v_blob(&WithChar { v: 'A' }), b"A");
}

/// A multi-byte char encodes as its UTF-8 bytes.
#[test]
#[ignore = "serialization not yet implemented"]
fn char_multibyte_textual_form() {
    assert_eq!(v_blob(&WithChar { v: 'é' }), "é".as_bytes());
}

// --- string ---

/// A String is stored as its UTF-8 bytes verbatim, including non-ASCII.
#[test]
#[ignore = "serialization not yet implemented"]
fn string_verbatim_utf8() {
    assert_eq!(
        v_blob(&WithString {
            v: "héllo".to_string()
        }),
        "héllo".as_bytes()
    );
}

/// A String is not quoted or escaped, and an embedded newline is preserved as-is
/// — there is no delimiter, quoting, or escaping in a leaf blob.
#[test]
#[ignore = "serialization not yet implemented"]
fn string_with_special_chars_not_quoted_or_escaped() {
    assert_eq!(
        v_blob(&WithString {
            v: "a\"b\nc".to_string()
        }),
        b"a\"b\nc"
    );
}

/// A leaf blob carries no trailing newline.
#[test]
#[ignore = "serialization not yet implemented"]
fn leaf_has_no_trailing_newline() {
    let bytes = v_blob(&WithString { v: "x".to_string() });
    assert_eq!(bytes, b"x");
    assert!(
        !bytes.ends_with(b"\n"),
        "leaf blob must not have a trailing newline"
    );
}
