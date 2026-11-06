//! The identity mini-codec and its schema-universe check.
//!
//! A [`NormalForm`] describes a value in a closed universe: fixed-width
//! scalars, bytes, sequences, named structs, and string-keyed maps. Values are
//! written with the same **bare value** encoding as [`crate::serialize`], not
//! the document's `.schema` envelope. Under the same schema and Git
//! hash format, the identity tree is therefore byte-for-byte identical to the
//! ordinary value tree.
//!
//! Text leaves contain canonical textual values followed by one newline;
//! [`NormalForm::Bytes`] and [`NormalForm::Hash`] are binary blobs with no
//! appended byte. Floats use the shared canonical formatter: both zeros have
//! the same representation, as do all NaNs. A hash is represented by its raw
//! bytes (like a byte sequence), not by its hexadecimal text.
//!
//! Sequences use three lowercase hexadecimal ordinal digits (`000`–`fff`).
//! Struct fields and string map keys use the format's canonical escaping, and
//! Git's comparator orders entries, including its implicit slash for trees.
//! Empty composites use the `none` marker tree. Every tree has at most 4096
//! entries and every encoded user name has at most 255 bytes.
//!
//! [`Key`] retains its other scalar variants for source compatibility, but a
//! map containing any such key is refused: the wire format supports only string
//! map keys. [`Node::Cbor`] is also refused by the universe check because the
//! codec does not yet implement canonical CBOR. The mapping is schema-directed
//! and untagged, so an object id identifies a value under a known shape.
//!
//! # Marking a subtree
//!
//! `#[facet(facet_git_tree::identity_key)]` on a field or type marks its
//! subtree (see [`crate::attr`]). [`schema_of`](crate::schema_of) compiles
//! the mark into a definition named with the reserved [`IDENTITY_DEF_PREFIX`]
//! (referenced by an ordinary [`Node::Ref`], adding no tree level);
//! [`identity_subtrees`] recovers the marked nodes and
//! [`check_identity_subtrees`] checks each.

use std::collections::BTreeMap;

use gix_object::Write;

use crate::error::{NormalFormError, SerializeError, UniverseError};
use crate::limits::{MAX_UNIVERSE_DEPTH, MAX_VALUE_DEPTH};
use crate::ordinal::{self, MAX_TREE_ENTRIES};
use crate::schema::{Node, Schema};
use crate::store::ObjectStore;
use crate::{EntryKind, EntryMode, ObjectId, TreeEntry};

/// The reserved prefix of a schema definition name holding an identity- or
/// key-bearing subtree.
///
/// `:` cannot occur in a Rust type identifier, so a reserved name can never
/// collide with a name [`schema_of`](crate::schema_of) assigns to a user type.
pub const IDENTITY_DEF_PREFIX: &str = "identity:";

/// A value encoded by the identity mini-codec.
///
/// This is a schema-free value, not a complete schema validation: callers must
/// supply every struct field and use a consistent shape for all list elements
/// and map values. Maps accept only [`Key::Str`]. Size, depth, and entry-name
/// limits are enforced when the value is written.
#[derive(Debug, Clone, PartialEq)]
pub enum NormalForm {
    /// A boolean.
    Bool(bool),
    /// An 8-bit signed integer.
    I8(i8),
    /// A 16-bit signed integer.
    I16(i16),
    /// A 32-bit signed integer.
    I32(i32),
    /// A 64-bit signed integer.
    I64(i64),
    /// A 128-bit signed integer.
    I128(i128),
    /// An 8-bit unsigned integer.
    U8(u8),
    /// A 16-bit unsigned integer.
    U16(u16),
    /// A 32-bit unsigned integer.
    U32(u32),
    /// A 64-bit unsigned integer.
    U64(u64),
    /// A 128-bit unsigned integer.
    U128(u128),
    /// A 32-bit float, using the canonical float text.
    F32(f32),
    /// A 64-bit float, using the canonical float text.
    F64(f64),
    /// A character.
    Char(char),
    /// A string.
    Str(String),
    /// A byte string, stored verbatim.
    Bytes(Vec<u8>),
    /// A Git object id, stored as its raw hash bytes.
    Hash(ObjectId),
    /// An ordered sequence or tuple; use [`Bytes`](Self::Bytes) for `u8` arrays.
    List(Vec<NormalForm>),
    /// A composite containing every field, keyed by its unescaped user name.
    Struct(BTreeMap<String, NormalForm>),
    /// A map keyed by strings, represented with [`Key::Str`].
    Map(BTreeMap<Key, NormalForm>),
}

/// A map key. Only [`Key::Str`] is supported by the frozen wire format.
///
/// The remaining variants are retained for compatibility and rejected at the
/// write boundary rather than silently encoding an unsupported map schema. An
/// empty [`NormalForm::Map`] carries no key-type information; schema-aware
/// callers must check its [`Node::Map`] key before writing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    /// An unsupported boolean key.
    Bool(bool),
    /// An unsupported 8-bit signed integer key.
    I8(i8),
    /// An unsupported 16-bit signed integer key.
    I16(i16),
    /// An unsupported 32-bit signed integer key.
    I32(i32),
    /// An unsupported 64-bit signed integer key.
    I64(i64),
    /// An unsupported 128-bit signed integer key.
    I128(i128),
    /// An unsupported 8-bit unsigned integer key.
    U8(u8),
    /// An unsupported 16-bit unsigned integer key.
    U16(u16),
    /// An unsupported 32-bit unsigned integer key.
    U32(u32),
    /// An unsupported 64-bit unsigned integer key.
    U64(u64),
    /// An unsupported 128-bit unsigned integer key.
    U128(u128),
    /// An unsupported character key.
    Char(char),
    /// A supported string key, escaped when written.
    Str(String),
    /// An unsupported byte-string key.
    Bytes(Vec<u8>),
    /// An unsupported Git object-id key.
    Hash(ObjectId),
}

impl Key {
    /// Return a string key's canonical escaped tree-entry name.
    ///
    /// Non-string keys and names exceeding the encoded 255-byte limit return
    /// [`NormalFormError::InvalidKey`].
    pub fn name(&self) -> Result<String, NormalFormError> {
        match self {
            Key::Str(name) => crate::name::escape(name)
                .map_err(|_error| NormalFormError::InvalidKey { key: name.clone() }),
            // `InvalidKey` carries a string, not a typed key; debug text
            // preserves the unsupported variant in the diagnostic.
            other => Err(NormalFormError::InvalidKey {
                key: format!("{other:?}"),
            }),
        }
    }
}

/// Write `value`'s bare Git objects into `store` and return the root object id.
///
/// All field names and every map key are validated before writing objects. A
/// schema-free value cannot prove that a struct has every schema field or that
/// a map is string-keyed when it is empty; use [`check_universe`] alongside
/// this function when a schema is available. `store` can be a `&dyn Write`.
///
/// The existing [`NormalFormError`] API has no variants for excessive depth
/// or oversized structs/maps. These errors are temporarily carried as a
/// [`NormalFormLimitError`] source of [`NormalFormError::Backend`].
pub fn hash_into<W: Write + ?Sized>(
    value: &NormalForm,
    store: &W,
) -> Result<ObjectId, NormalFormError> {
    validate_value(value, 0)?;
    write_node(value, store).map(|(oid, _kind)| oid)
}

/// [`hash_into`] against a fresh default (SHA-1) [`ObjectStore`], returned
/// alongside the root id so a caller can relay the objects onward.
///
/// Use [`hash_into`] with a store configured for another Git hash format when
/// necessary. Object ids are only comparable within the same hash format.
pub fn hash(value: &NormalForm) -> Result<(ObjectId, ObjectStore), NormalFormError> {
    let store = ObjectStore::default();
    let root = hash_into(value, &store)?;
    Ok((root, store))
}

/// Translate the shared writing helpers' backend errors to this API's error.
/// Those helpers perform no validation or reflection; `Backend` is their only
/// current failure. Preserve any future error as a source rather than dropping
/// it or panicking.
fn write_error(error: SerializeError) -> NormalFormError {
    match error {
        SerializeError::Backend(source) => NormalFormError::Backend(source),
        other => NormalFormError::Backend(Box::new(other)),
    }
}

/// A depth or non-list tree-size violation carried by the existing
/// [`NormalFormError::Backend`] variant until that error API has dedicated
/// limit variants. Its source can be downcast to distinguish validation
/// failure from an actual backend error.
#[derive(Debug, thiserror::Error)]
pub enum NormalFormLimitError {
    /// A struct or map has too many entries.
    #[error("normal-form tree holds {len} entries, exceeding the limit of {max}")]
    TreeTooLong {
        /// The number of entries supplied.
        len: usize,
        /// The maximum permitted number of entries.
        max: usize,
    },
    /// A value exceeds the maximum number of nested objects.
    #[error("normal-form value exceeds the maximum depth of {0}")]
    MaxDepth(usize),
}

fn check_tree_len(len: usize) -> Result<(), NormalFormError> {
    if len > MAX_TREE_ENTRIES {
        return Err(NormalFormError::Backend(Box::new(
            NormalFormLimitError::TreeTooLong {
                len,
                max: MAX_TREE_ENTRIES,
            },
        )));
    }
    Ok(())
}

/// Validate the complete value before writing, so invalid keys, names, sizes,
/// and depth cannot leave partial value objects in the store.
fn validate_value(value: &NormalForm, depth: usize) -> Result<(), NormalFormError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(NormalFormError::Backend(Box::new(
            NormalFormLimitError::MaxDepth(MAX_VALUE_DEPTH),
        )));
    }
    match value {
        NormalForm::List(items) => {
            if items.len() > MAX_TREE_ENTRIES {
                return Err(NormalFormError::ListTooLong {
                    len: items.len(),
                    max: MAX_TREE_ENTRIES,
                });
            }
            for item in items {
                validate_value(item, depth + 1)?;
            }
        }
        NormalForm::Struct(fields) => {
            check_tree_len(fields.len())?;
            for (name, value) in fields {
                crate::name::escape(name).map_err(|_error| NormalFormError::InvalidFieldName {
                    field: name.clone(),
                })?;
                validate_value(value, depth + 1)?;
            }
        }
        NormalForm::Map(pairs) => {
            check_tree_len(pairs.len())?;
            for (key, value) in pairs {
                key.name()?;
                validate_value(value, depth + 1)?;
            }
        }
        NormalForm::Bool(_)
        | NormalForm::I8(_)
        | NormalForm::I16(_)
        | NormalForm::I32(_)
        | NormalForm::I64(_)
        | NormalForm::I128(_)
        | NormalForm::U8(_)
        | NormalForm::U16(_)
        | NormalForm::U32(_)
        | NormalForm::U64(_)
        | NormalForm::U128(_)
        | NormalForm::F32(_)
        | NormalForm::F64(_)
        | NormalForm::Char(_)
        | NormalForm::Str(_)
        | NormalForm::Bytes(_)
        | NormalForm::Hash(_) => {}
    }
    Ok(())
}

fn write_node<W: Write + ?Sized>(
    value: &NormalForm,
    store: &W,
) -> Result<(ObjectId, EntryKind), NormalFormError> {
    match value {
        NormalForm::List(items) => {
            let entries = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let name =
                        ordinal::encode(index).map_err(|_error| NormalFormError::ListTooLong {
                            len: items.len(),
                            max: MAX_TREE_ENTRIES,
                        })?;
                    entry(name, item, store)
                })
                .collect::<Result<Vec<_>, _>>()?;
            write_tree(entries, store)
        }
        NormalForm::Struct(fields) => {
            let entries = fields
                .iter()
                .map(|(name, field)| {
                    let escaped = crate::name::escape(name).map_err(|_error| {
                        NormalFormError::InvalidFieldName {
                            field: name.clone(),
                        }
                    })?;
                    entry(escaped, field, store)
                })
                .collect::<Result<Vec<_>, _>>()?;
            write_tree(entries, store)
        }
        NormalForm::Map(pairs) => {
            let entries = pairs
                .iter()
                .map(|(key, item)| entry(key.name()?, item, store))
                .collect::<Result<Vec<_>, _>>()?;
            write_tree(entries, store)
        }
        NormalForm::Bytes(bytes) => binary_blob(bytes, store),
        NormalForm::Hash(hash) => binary_blob(hash.as_bytes(), store),
        NormalForm::Bool(v) => text_blob(&v.to_string(), store),
        NormalForm::I8(v) => text_blob(&v.to_string(), store),
        NormalForm::I16(v) => text_blob(&v.to_string(), store),
        NormalForm::I32(v) => text_blob(&v.to_string(), store),
        NormalForm::I64(v) => text_blob(&v.to_string(), store),
        NormalForm::I128(v) => text_blob(&v.to_string(), store),
        NormalForm::U8(v) => text_blob(&v.to_string(), store),
        NormalForm::U16(v) => text_blob(&v.to_string(), store),
        NormalForm::U32(v) => text_blob(&v.to_string(), store),
        NormalForm::U64(v) => text_blob(&v.to_string(), store),
        NormalForm::U128(v) => text_blob(&v.to_string(), store),
        NormalForm::F32(v) => text_blob(&crate::float::format_f32(*v), store),
        NormalForm::F64(v) => text_blob(&crate::float::format_f64(*v), store),
        NormalForm::Char(v) => text_blob(&v.to_string(), store),
        NormalForm::Str(v) => text_blob(v, store),
    }
}

fn text_blob<W: Write + ?Sized>(
    text: &str,
    store: &W,
) -> Result<(ObjectId, EntryKind), NormalFormError> {
    let oid = crate::ser::write_leaf_blob(store, text.as_bytes()).map_err(write_error)?;
    Ok((oid, EntryKind::Blob))
}

fn binary_blob<W: Write + ?Sized>(
    bytes: &[u8],
    store: &W,
) -> Result<(ObjectId, EntryKind), NormalFormError> {
    let oid = crate::ser::write_binary_blob(store, bytes).map_err(write_error)?;
    Ok((oid, EntryKind::Blob))
}

fn entry<W: Write + ?Sized>(
    name: String,
    value: &NormalForm,
    store: &W,
) -> Result<TreeEntry, NormalFormError> {
    let (oid, kind) = write_node(value, store)?;
    Ok(TreeEntry {
        mode: EntryMode::from(kind),
        filename: name.into(),
        oid,
    })
}

fn write_tree<W: Write + ?Sized>(
    mut entries: Vec<TreeEntry>,
    store: &W,
) -> Result<(ObjectId, EntryKind), NormalFormError> {
    if entries.is_empty() {
        let oid = crate::marker::write_marker_tree(store).map_err(write_error)?;
        return Ok((oid, EntryKind::Tree));
    }
    // `TreeEntry::Ord` is Git's comparator; unlike string sorting it treats
    // a tree name as ending with `/` when another name shares its prefix.
    entries.sort();
    let oid = store
        .write(&gix_object::Tree { entries })
        .map_err(NormalFormError::Backend)?;
    Ok((oid, EntryKind::Tree))
}

/// Every identity- or key-bearing subtree of `schema`: its definition name
/// and the marked node.
pub fn identity_subtrees(schema: &Schema) -> impl Iterator<Item = (&str, &Node)> {
    schema
        .defs
        .iter()
        .filter(|(name, _)| name.starts_with(IDENTITY_DEF_PREFIX))
        .map(|(name, node)| (name.as_str(), node))
}

/// Check every identity- or key-bearing subtree of `schema` against the
/// mini-codec's universe.
pub fn check_identity_subtrees(schema: &Schema) -> Result<(), UniverseError> {
    for (name, node) in identity_subtrees(schema) {
        check_universe_at(node, &schema.defs, name)?;
    }
    Ok(())
}

/// Check whether `node`, resolved through `defs`, is in the identity universe.
///
/// A map key must be the direct [`Node::String`] node. The checker refuses
/// [`Node::Cbor`], because canonical CBOR is not implemented, as well as enums,
/// dynamic values, raw trees, options, unit and platform-width integers. It
/// also refuses fields or fixed-size containers that cannot fit the wire
/// limits. References are resolved with a depth bound, so dangling or cyclic
/// references cannot be silently accepted.
pub fn check_universe(node: &Node, defs: &BTreeMap<String, Node>) -> Result<(), UniverseError> {
    check_universe_at(node, defs, "")
}

/// [`check_universe`] with `root` naming the subtree in reported paths.
pub(crate) fn check_universe_at(
    node: &Node,
    defs: &BTreeMap<String, Node>,
    root: &str,
) -> Result<(), UniverseError> {
    walk(node, defs, root.to_owned(), 0)
}

fn walk(
    node: &Node,
    defs: &BTreeMap<String, Node>,
    path: String,
    depth: usize,
) -> Result<(), UniverseError> {
    if depth > MAX_UNIVERSE_DEPTH {
        return Err(UniverseError::MaxDepth {
            path,
            depth: MAX_UNIVERSE_DEPTH,
        });
    }
    let excluded = |found: &'static str| UniverseError::Excluded {
        path: path.clone(),
        found,
    };
    match node {
        Node::Bool
        | Node::Char
        | Node::String
        | Node::I8
        | Node::I16
        | Node::I32
        | Node::I64
        | Node::I128
        | Node::U8
        | Node::U16
        | Node::U32
        | Node::U64
        | Node::U128
        | Node::F32
        | Node::F64
        | Node::Bytes => Ok(()),
        Node::Struct(fields) => {
            if fields.len() > MAX_TREE_ENTRIES {
                return Err(excluded("oversized Struct"));
            }
            for (name, field) in fields {
                let field_path = format!("{path}.{name}");
                if crate::name::escape(name).is_err() {
                    return Err(UniverseError::Excluded {
                        path: field_path,
                        found: "overlong field name",
                    });
                }
                walk(field, defs, field_path, depth + 1)?;
            }
            Ok(())
        }
        Node::Tuple(elems) => {
            if elems.len() > MAX_TREE_ENTRIES {
                return Err(excluded("oversized Tuple"));
            }
            elems
                .iter()
                .enumerate()
                .try_for_each(|(i, elem)| walk(elem, defs, format!("{path}.{i}"), depth + 1))
        }
        Node::List(elem) => walk(elem, defs, format!("{path}[]"), depth + 1),
        Node::Array { elem, len } => {
            if *len > MAX_TREE_ENTRIES {
                return Err(excluded("oversized Array"));
            }
            walk(elem, defs, format!("{path}[]"), depth + 1)
        }
        Node::Map { key, value } => {
            if !matches!(key.as_ref(), Node::String) {
                return Err(UniverseError::Excluded {
                    path: format!("{path}<key>"),
                    found: node_name(key),
                });
            }
            walk(value, defs, format!("{path}{{}}"), depth + 1)
        }
        Node::Ref(name) => {
            let def = defs.get(name).ok_or_else(|| UniverseError::UnknownRef {
                path: path.clone(),
                name: name.clone(),
            })?;
            walk(def, defs, path, depth + 1)
        }
        Node::Unit => Err(excluded("Unit")),
        Node::ISize => Err(excluded("ISize")),
        Node::USize => Err(excluded("USize")),
        Node::Cbor => Err(excluded("Cbor")),
        Node::Optional(_) => Err(excluded("Optional")),
        Node::Enum(_) => Err(excluded("Enum")),
        Node::RawTree => Err(excluded("RawTree")),
        Node::Dynamic => Err(excluded("Dynamic")),
    }
}

/// The [`Node`] variant's name, for an error to report.
fn node_name(node: &Node) -> &'static str {
    match node {
        Node::Unit => "Unit",
        Node::Bool => "Bool",
        Node::Char => "Char",
        Node::String => "String",
        Node::I8 => "I8",
        Node::I16 => "I16",
        Node::I32 => "I32",
        Node::I64 => "I64",
        Node::I128 => "I128",
        Node::ISize => "ISize",
        Node::U8 => "U8",
        Node::U16 => "U16",
        Node::U32 => "U32",
        Node::U64 => "U64",
        Node::U128 => "U128",
        Node::USize => "USize",
        Node::F32 => "F32",
        Node::F64 => "F64",
        Node::Bytes => "Bytes",
        Node::Cbor => "Cbor",
        Node::Struct(_) => "Struct",
        Node::Tuple(_) => "Tuple",
        Node::List(_) => "List",
        Node::Array { .. } => "Array",
        Node::Map { .. } => "Map",
        Node::Optional(_) => "Optional",
        Node::Enum(_) => "Enum",
        Node::RawTree => "RawTree",
        Node::Dynamic => "Dynamic",
        Node::Ref(_) => "Ref",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use facet::Facet;
    use gix_hash::Kind as HashKind;

    fn blob(value: NormalForm) -> Vec<u8> {
        let (oid, store) = hash(&value).expect("write normal form");
        store
            .get_blob(&oid)
            .expect("read blob")
            .expect("blob exists")
    }

    fn tree_names(value: NormalForm) -> Vec<String> {
        let (oid, store) = hash(&value).expect("write normal form");
        store
            .get_tree(&oid)
            .expect("read tree")
            .expect("tree exists")
            .into_iter()
            .map(|entry| entry.filename.to_string())
            .collect()
    }

    fn assert_typed_matches<T: for<'a> Facet<'a>>(value: &T, normal: &NormalForm) {
        for kind in [HashKind::Sha1, HashKind::Sha256] {
            let store = ObjectStore::with_hash_kind(kind);
            let typed = crate::serialize_into(value, &store).expect("write typed value");
            let identity = hash_into(normal, &store).expect("write identity value");
            assert_eq!(typed, identity, "{kind:?}: {normal:?}");
        }
    }

    #[test]
    fn canonical_text_leaves_and_raw_binary() {
        let cases = [
            (NormalForm::Bool(false), b"false\n".as_slice()),
            (NormalForm::Bool(true), b"true\n".as_slice()),
            (NormalForm::I8(i8::MIN), b"-128\n".as_slice()),
            (NormalForm::I16(i16::MAX), b"32767\n".as_slice()),
            (NormalForm::I32(-31), b"-31\n".as_slice()),
            (NormalForm::I64(0), b"0\n".as_slice()),
            (
                NormalForm::I128(i128::MIN),
                b"-170141183460469231731687303715884105728\n".as_slice(),
            ),
            (NormalForm::U8(u8::MAX), b"255\n".as_slice()),
            (NormalForm::U16(10), b"10\n".as_slice()),
            (NormalForm::U32(4630), b"4630\n".as_slice()),
            (NormalForm::U64(0), b"0\n".as_slice()),
            (
                NormalForm::U128(u128::MAX),
                b"340282366920938463463374607431768211455\n".as_slice(),
            ),
            (NormalForm::Char('é'), "é\n".as_bytes()),
            (NormalForm::Char('\n'), b"\n\n".as_slice()),
            (NormalForm::Str(String::new()), b"\n".as_slice()),
            (NormalForm::Str("ab\n".into()), b"ab\n\n".as_slice()),
            (NormalForm::Bytes(Vec::new()), b"".as_slice()),
            (NormalForm::Bytes(vec![0, 10, 255]), b"\0\n\xff".as_slice()),
        ];
        for (value, expected) in cases {
            assert_eq!(blob(value), expected);
        }

        let hash = ObjectId::from_hex(b"00112233445566778899aabbccddeeff00112233")
            .expect("valid Git object id");
        assert_eq!(blob(NormalForm::Hash(hash)), hash.as_bytes());
        assert_typed_matches(&hash.as_bytes().to_vec(), &NormalForm::Hash(hash));
    }

    #[test]
    fn floats_use_same_canonical_formatter_as_full_encoding() {
        for (value, expected) in [
            (0.0_f32, "0.0\n"),
            (-0.0_f32, "0.0\n"),
            (1.0_f32, "1.0\n"),
            (1.0e-7_f32, "1e-7\n"),
            (f32::INFINITY, "inf\n"),
            (f32::NEG_INFINITY, "-inf\n"),
            (f32::from_bits(0xffc0_1234), "nan\n"),
        ] {
            let normal = NormalForm::F32(value);
            assert_eq!(blob(normal.clone()), expected.as_bytes());
            assert_typed_matches(&value, &normal);
        }
        for (value, expected) in [
            (0.0_f64, "0.0\n"),
            (-0.0_f64, "0.0\n"),
            (1.5_f64, "1.5\n"),
            (1.0e21_f64, "1e21\n"),
            (f64::INFINITY, "inf\n"),
            (f64::NEG_INFINITY, "-inf\n"),
            (f64::from_bits(0xfff8_0000_0000_1234), "nan\n"),
        ] {
            let normal = NormalForm::F64(value);
            assert_eq!(blob(normal.clone()), expected.as_bytes());
            assert_typed_matches(&value, &normal);
        }
    }

    #[test]
    fn names_are_escaped_and_tree_order_uses_git_comparison() {
        let fields = BTreeMap::from([
            ("a".into(), NormalForm::Struct(BTreeMap::new())),
            ("a.b".into(), NormalForm::U8(1)),
            ("none".into(), NormalForm::Str("n".into())),
            (".schema".into(), NormalForm::Bool(false)),
            ("a/b".into(), NormalForm::Bool(true)),
            ("naïve".into(), NormalForm::Bytes(Vec::new())),
            (String::new(), NormalForm::U8(0)),
        ]);
        assert_eq!(
            tree_names(NormalForm::Struct(fields)),
            [
                "%",
                "%.schema",
                "%a%2Fb",
                "%na%C3%AFve",
                "%none",
                "a.b",
                "a"
            ]
        );
        assert_eq!(Key::Str("".into()).name().expect("valid key"), "%");
        assert_eq!(Key::Str("some".into()).name().expect("valid key"), "%some");
        assert_eq!(Key::Str("a/b".into()).name().expect("valid key"), "%a%2Fb");
        assert!(matches!(
            Key::U8(7).name(),
            Err(NormalFormError::InvalidKey { .. })
        ));

        let maximal = format!("{}ab", "é".repeat(42));
        let encoded = Key::Str(maximal).name().expect("255 encoded bytes fit");
        assert_eq!(encoded.len(), 255);
        assert_eq!(
            tree_names(NormalForm::Map(BTreeMap::from([(
                Key::Str("é".repeat(42) + "ab"),
                NormalForm::Bool(true)
            )]))),
            [encoded]
        );
    }

    #[test]
    fn all_empty_composites_use_the_same_nonempty_marker_as_typed_values() {
        let empty_list = NormalForm::List(Vec::new());
        let empty_struct = NormalForm::Struct(BTreeMap::new());
        let empty_map = NormalForm::Map(BTreeMap::new());
        let (marker, store) = hash(&empty_list).expect("write marker");
        let entries = store
            .get_tree(&marker)
            .expect("read tree")
            .expect("marker exists");
        assert_eq!(entries.len(), 1);
        assert_eq!(&entries[0].filename[..], b"none");
        assert_eq!(entries[0].mode, EntryMode::from(EntryKind::Blob));
        assert_eq!(
            store.get_blob(&entries[0].oid).expect("read marker blob"),
            Some(Vec::new())
        );
        assert_eq!(hash(&empty_struct).expect("struct marker").0, marker);
        assert_eq!(hash(&empty_map).expect("map marker").0, marker);
        assert_typed_matches(&Vec::<u16>::new(), &empty_list);
        assert_typed_matches(&BTreeMap::<String, bool>::new(), &empty_map);
        #[derive(Facet)]
        struct Empty {}
        assert_typed_matches(&Empty {}, &empty_struct);
    }

    #[test]
    fn hex_ordinals_include_full_cap_and_exceeding_it_fails() {
        let normal = NormalForm::List(vec![NormalForm::U16(7); MAX_TREE_ENTRIES]);
        let names = tree_names(normal.clone());
        assert_eq!(names.len(), MAX_TREE_ENTRIES);
        for (index, expected) in [
            (0, "000"),
            (9, "009"),
            (10, "00a"),
            (16, "010"),
            (4095, "fff"),
        ] {
            assert_eq!(names[index], expected);
        }
        assert_typed_matches(&vec![7_u16; MAX_TREE_ENTRIES], &normal);
        assert!(matches!(
            hash(&NormalForm::List(vec![NormalForm::U8(0); MAX_TREE_ENTRIES + 1])),
            Err(NormalFormError::ListTooLong { len, max })
                if len == MAX_TREE_ENTRIES + 1 && max == MAX_TREE_ENTRIES
        ));
    }

    #[test]
    fn name_and_entry_caps_are_checked_before_writing_anything() {
        let long = "a".repeat(256);
        let expanded_too_long = format!("{}abc", "é".repeat(42));
        let store = ObjectStore::default();
        for name in [long, expanded_too_long] {
            let structure =
                NormalForm::Struct(BTreeMap::from([(name.clone(), NormalForm::Bool(true))]));
            assert!(
                matches!(hash_into(&structure, &store), Err(NormalFormError::InvalidFieldName { field }) if field == name)
            );
            let map = NormalForm::Map(BTreeMap::from([(
                Key::Str(name.clone()),
                NormalForm::Bool(true),
            )]));
            assert!(
                matches!(hash_into(&map, &store), Err(NormalFormError::InvalidKey { key }) if key == name)
            );
        }
        let fields = (0..=MAX_TREE_ENTRIES)
            .map(|index| (format!("k{index}"), NormalForm::Bool(true)))
            .collect();
        assert!(matches!(
            hash_into(&NormalForm::Struct(fields), &store),
            Err(NormalFormError::Backend(source))
                if matches!(source.downcast_ref::<NormalFormLimitError>(),
                    Some(NormalFormLimitError::TreeTooLong { len, max })
                        if *len == MAX_TREE_ENTRIES + 1 && *max == MAX_TREE_ENTRIES)
        ));
        let pairs = (0..=MAX_TREE_ENTRIES)
            .map(|index| (Key::Str(format!("k{index}")), NormalForm::Bool(true)))
            .collect();
        assert!(matches!(
            hash_into(&NormalForm::Map(pairs), &store),
            Err(NormalFormError::Backend(source))
                if matches!(source.downcast_ref::<NormalFormLimitError>(),
                    Some(NormalFormLimitError::TreeTooLong { len, max })
                        if *len == MAX_TREE_ENTRIES + 1 && *max == MAX_TREE_ENTRIES)
        ));
        let leaf = crate::serialize(&true).expect("typed leaf").0;
        assert_eq!(store.get(&leaf).expect("store lookup"), None);
    }

    #[test]
    fn capped_struct_and_map_match_the_full_codec() {
        let fields: BTreeMap<String, u16> = (0..MAX_TREE_ENTRIES)
            .map(|index| (format!("k{index}"), 7))
            .collect();
        let normal_map = NormalForm::Map(
            fields
                .iter()
                .map(|(name, value)| (Key::Str(name.clone()), NormalForm::U16(*value)))
                .collect(),
        );
        let normal_struct = NormalForm::Struct(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), NormalForm::U16(*value)))
                .collect(),
        );
        let names = tree_names(normal_map.clone());
        assert_eq!(names.len(), MAX_TREE_ENTRIES);
        assert_typed_matches(&fields, &normal_map);
        assert_eq!(
            hash(&normal_map).expect("capped map").0,
            hash(&normal_struct).expect("capped struct").0,
        );
    }

    #[test]
    fn exact_depth_limit_and_unsupported_map_keys_are_checked() {
        let mut nested = NormalForm::Bool(true);
        for _ in 0..MAX_VALUE_DEPTH {
            nested = NormalForm::List(vec![nested]);
        }
        hash(&nested).expect("maximum depth is permitted");
        let too_deep = NormalForm::List(vec![nested]);
        assert!(matches!(
            hash(&too_deep),
            Err(NormalFormError::Backend(source))
                if matches!(source.downcast_ref::<NormalFormLimitError>(),
                    Some(NormalFormLimitError::MaxDepth(MAX_VALUE_DEPTH)))
        ));
        let unsupported = NormalForm::Map(BTreeMap::from([
            (Key::Bool(true), NormalForm::U8(0)),
            (Key::Str("true".into()), NormalForm::U8(1)),
        ]));
        assert!(matches!(
            hash(&unsupported),
            Err(NormalFormError::InvalidKey { .. })
        ));
    }

    #[test]
    fn complex_identity_matches_the_full_codec_in_both_hash_formats() {
        #[derive(Facet)]
        struct Identity {
            #[facet(rename = "a/b")]
            path: String,
            #[facet(rename = "none")]
            #[facet(default)]
            payload: Vec<u8>,
            a: BTreeMap<String, u64>,
            #[facet(rename = "a.b")]
            span: (u64, u64),
            reading: f32,
        }
        let value = Identity {
            path: "hello\nworld".into(),
            payload: Vec::new(),
            a: BTreeMap::from([
                (String::new(), 1),
                ("some".into(), 2),
                ("naïve".into(), 3),
                ("a/b".into(), 4),
            ]),
            span: (5, 6),
            reading: -0.0,
        };
        let normal = NormalForm::Struct(BTreeMap::from([
            ("a/b".into(), NormalForm::Str(value.path.clone())),
            ("none".into(), NormalForm::Bytes(value.payload.clone())),
            (
                "a".into(),
                NormalForm::Map(
                    value
                        .a
                        .iter()
                        .map(|(key, val)| (Key::Str(key.clone()), NormalForm::U64(*val)))
                        .collect(),
                ),
            ),
            (
                "a.b".into(),
                NormalForm::List(vec![NormalForm::U64(5), NormalForm::U64(6)]),
            ),
            ("reading".into(), NormalForm::F32(value.reading)),
        ]));
        assert_typed_matches(&value, &normal);
        let doc = crate::schema_of::<Identity>().expect("identity schema");
        check_universe(&doc.root, &doc.defs).expect("identity schema is supported");
    }

    #[test]
    fn universe_rejects_cbor_wrong_map_keys_and_unsupported_shapes() {
        let defs = BTreeMap::from([
            ("Str".into(), Node::String),
            ("loop".into(), Node::Ref("loop".into())),
        ]);
        let excluded = |node: &Node| match check_universe(node, &defs) {
            Err(UniverseError::Excluded { found, .. }) => found,
            other => panic!("expected an exclusion, got {other:?}"),
        };
        assert_eq!(excluded(&Node::Cbor), "Cbor");
        assert_eq!(
            excluded(&Node::Optional(Box::new(Node::String))),
            "Optional"
        );
        assert_eq!(excluded(&Node::Dynamic), "Dynamic");
        assert_eq!(excluded(&Node::RawTree), "RawTree");
        assert_eq!(excluded(&Node::Unit), "Unit");
        assert_eq!(excluded(&Node::ISize), "ISize");
        assert_eq!(excluded(&Node::USize), "USize");
        assert_eq!(excluded(&Node::Enum(BTreeMap::new())), "Enum");
        for (key, expected) in [
            (Node::U8, "U8"),
            (Node::Cbor, "Cbor"),
            (Node::Ref("Str".into()), "Ref"),
        ] {
            let schema = Node::Map {
                key: Box::new(key),
                value: Box::new(Node::String),
            };
            assert_eq!(excluded(&schema), expected);
        }
        assert!(
            check_universe(
                &Node::Map {
                    key: Box::new(Node::String),
                    value: Box::new(Node::U16)
                },
                &defs
            )
            .is_ok()
        );
        assert!(
            matches!(check_universe(&Node::Ref("missing".into()), &defs), Err(UniverseError::UnknownRef { name, .. }) if name == "missing")
        );
        assert!(matches!(
            check_universe(&Node::Ref("loop".into()), &defs),
            Err(UniverseError::MaxDepth { .. })
        ));
    }

    #[test]
    fn universe_follows_direct_struct_nodes_and_checks_schema_caps() {
        let defs = BTreeMap::new();
        let schema = Node::Struct(BTreeMap::from([
            ("a/b".into(), Node::Bool),
            (
                "nested".into(),
                Node::Struct(BTreeMap::from([(
                    "missing".into(),
                    Node::Ref("absent".into()),
                )])),
            ),
        ]));
        assert!(
            matches!(check_universe_at(&schema, &defs, "id"), Err(UniverseError::UnknownRef { path, name }) if path == "id.nested.missing" && name == "absent")
        );
        let overlong = Node::Struct(BTreeMap::from([("é".repeat(43), Node::Bool)]));
        assert!(matches!(
            check_universe(&overlong, &defs),
            Err(UniverseError::Excluded {
                found: "overlong field name",
                ..
            })
        ));
        let large_fields = (0..=MAX_TREE_ENTRIES)
            .map(|i| (format!("f{i}"), Node::Bool))
            .collect();
        let oversized = [
            Node::Struct(large_fields),
            Node::Tuple(vec![Node::Bool; MAX_TREE_ENTRIES + 1]),
            Node::Array {
                elem: Box::new(Node::Bool),
                len: MAX_TREE_ENTRIES + 1,
            },
        ];
        for schema in oversized {
            assert!(matches!(
                check_universe(&schema, &defs),
                Err(UniverseError::Excluded { .. })
            ));
        }
        let largest = Node::Array {
            elem: Box::new(Node::Bool),
            len: MAX_TREE_ENTRIES,
        };
        assert!(check_universe(&largest, &defs).is_ok());
    }
}
