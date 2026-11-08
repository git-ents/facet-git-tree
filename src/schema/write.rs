//! Schema-directed serialization: encoding a full-fidelity
//! [`facet_value::Value`] as a Git object guided by a [`Schema`].
//!
//! The write-side mirror of [`schema::read`](crate::schema::read). A dynamic
//! value parsed from JSON does not, on its own, know which typed encoding a
//! field expects — a bare string cannot tell that its field is an `Option`
//! whose `Some` payload must be wrapped in a `some` entry, and a bare number
//! cannot tell that its field is `f64` rather than an integer. The schema
//! supplies exactly that missing type information, so
//! `serialize_value_with_schema` writes the same objects — byte-for-byte,
//! and therefore the same object ids — that the equivalent typed value would
//! produce through [`serialize`](crate::serialize).
//!
//! Encoding validates the schema and value. JSON-authored values may use
//! lossless integers and the strings `nan`, `inf`, and `-inf` for floats,
//! and UTF-8 strings for `Bytes`. CBOR nodes require opaque canonical bytes.

use std::collections::BTreeMap;

use facet::Peek;
use facet_value::{VArray, VNumber, Value};
use gix_object::Write;

use crate::error::{SchemaWriteError, SerializeError};
use crate::float::{format_f32, format_f64};
use crate::hash::{HashFormat, Oid, Tagged};
use crate::limits::MAX_VALUE_DEPTH;
use crate::ordinal::{self, MAX_TREE_ENTRIES};
use crate::schema::path::Path;
use crate::schema::{CheckedSchema, Node, RefError, Schema, VariantKind, value_kind};
use crate::ser::{serialize_node, wrap_oid, write_binary_blob, write_leaf_blob};
use crate::{EntryKind, EntryMode, ObjectId, TreeEntry};

/// Serialize the bare value described by `doc` into `store`.
///
/// The returned object may be a tree or a blob; no `.schema` envelope is
/// added. The document reached here has already passed the full
/// well-formedness walk that produced the [`CheckedSchema`]; it is not
/// re-checked. A `Cbor` value must be bytes containing exactly one canonical
/// CBOR item; those bytes are stored verbatim, without a binary or newline
/// wrapper. An unwrapped null denotes `None`, so `Some(None)` and `Some(())`
/// cannot be represented through this `Value` API. `store` is any `gix`
/// [`Write`] sink, as for [`serialize_into`](crate::serialize_into). Value
/// mismatches identify the offending path.
///
/// ```
/// use facet::Facet;
/// use facet_git_tree::{schema_of, serialize, serialize_value_with_schema};
/// use facet_value::value;
///
/// #[derive(Facet)]
/// struct Point {
///     x: f64,
///     y: f64,
/// }
///
/// let doc = schema_of::<Point>()?;
/// let checked = doc.checked()?;
/// let store = facet_git_tree::ObjectStore::default();
/// let via_schema = serialize_value_with_schema(&value!({ "x": 1.0, "y": 2.0 }), &checked, &store)?;
///
/// // Identical to the typed encoding, down to the object id.
/// let (typed, _) = serialize(&Point { x: 1.0, y: 2.0 })?;
/// assert_eq!(via_schema, typed);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn serialize_value_with_schema<F: HashFormat, W: Write + Tagged<F> + ?Sized>(
    value: &Value,
    doc: &CheckedSchema<'_>,
    store: &W,
) -> Result<Oid<F>, SchemaWriteError> {
    let schema = doc.schema();
    let (oid, _kind) = write_node(value, schema.root(), schema, store, &Path::ROOT, 0)?;
    Ok(oid)
}

/// Write one schema node's value from `value`.
///
/// Returns the encoded object's id and its entry kind (blob vs. tree), so a
/// caller embedding it in a parent tree can set the entry mode exactly as the
/// typed encoder does.
///
/// `depth` counts encoded nesting levels against [`MAX_VALUE_DEPTH`];
/// resolving a [`Node::Ref`] does not add a tree level.
fn write_node<F: HashFormat, W: Write + Tagged<F> + ?Sized>(
    value: &Value,
    schema: &Node,
    doc: &Schema,
    store: &W,
    path: &Path,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    check_depth(path, depth)?;
    match schema {
        Node::Unit => {
            if value.is_null() {
                marker_tree(store)
            } else {
                Err(expected(path, "null", value))
            }
        }
        Node::Bool => match value.as_bool() {
            Some(true) => blob(store, b"true"),
            Some(false) => blob(store, b"false"),
            None => Err(expected(path, "bool", value)),
        },
        Node::Char => {
            let mut buf = [0u8; 4];
            if let Some(c) = value.as_char() {
                blob(store, c.encode_utf8(&mut buf).as_bytes())
            } else if let Some(c) = value_str(value).and_then(single_char) {
                blob(store, c.encode_utf8(&mut buf).as_bytes())
            } else {
                Err(expected(path, "char or single-character string", value))
            }
        }
        Node::String => match value_str(value) {
            Some(s) => blob(store, s.as_bytes()),
            None => Err(expected(path, "string", value)),
        },
        Node::I8 => signed_blob::<i8, _, _>(value, "I8", path, store),
        Node::I16 => signed_blob::<i16, _, _>(value, "I16", path, store),
        Node::I32 => signed_blob::<i32, _, _>(value, "I32", path, store),
        Node::I64 => signed_blob::<i64, _, _>(value, "I64", path, store),
        Node::I128 => signed_blob::<i128, _, _>(value, "I128", path, store),
        // `isize`/`usize` are spec'd i64/u64-shaped, so the schema-directed
        // writer renders them at the fixed width: the blob is target-
        // independent by construction, not by fixture discipline.
        Node::ISize => signed_blob::<i64, _, _>(value, "ISize", path, store),
        Node::U8 => unsigned_blob::<u8, _, _>(value, "U8", path, store),
        Node::U16 => unsigned_blob::<u16, _, _>(value, "U16", path, store),
        Node::U32 => unsigned_blob::<u32, _, _>(value, "U32", path, store),
        Node::U64 => unsigned_blob::<u64, _, _>(value, "U64", path, store),
        Node::U128 => unsigned_blob::<u128, _, _>(value, "U128", path, store),
        Node::USize => unsigned_blob::<u64, _, _>(value, "USize", path, store),
        Node::F64 => {
            let f = float64(value, path)?;
            blob(store, format_f64(f).as_bytes())
        }
        Node::F32 => {
            let f = float32(value, path)?;
            blob(store, format_f32(f).as_bytes())
        }
        // `Bytes` reads back as raw bytes; a JSON author who cannot express a
        // byte string writes an ordinary string, whose UTF-8 bytes become the
        // blob — the same blob a typed `Vec<u8>` of those bytes produces.
        Node::Bytes => {
            if let Some(b) = value.as_bytes() {
                binary_blob(store, b.as_slice())
            } else if let Some(s) = value_str(value) {
                binary_blob(store, s.as_bytes())
            } else {
                Err(expected(path, "bytes or string", value))
            }
        }
        Node::Cbor => {
            let bytes = value
                .as_bytes()
                .ok_or_else(|| expected(path, "canonical CBOR bytes", value))?;
            crate::cbor::validate(bytes.as_slice()).map_err(|error| {
                SerializeError::Reflect(format!("at {}: invalid CBOR: {error}", path.show()))
            })?;
            binary_blob(store, bytes.as_slice())
        }
        Node::Struct(fields) => write_named_tree(value, fields, doc, store, path, depth),
        Node::Tuple(elems) => {
            let arr = as_array(value, path)?;
            if arr.len() != elems.len() {
                return Err(length_mismatch(path, elems.len(), arr.len()));
            }
            write_seq(arr, |i| &elems[i], doc, store, path, depth)
        }
        Node::List(elem) => {
            let arr = as_array(value, path)?;
            write_seq(arr, |_| elem, doc, store, path, depth)
        }
        Node::Array { elem, len } => {
            let arr = as_array(value, path)?;
            if arr.len() != *len {
                return Err(length_mismatch(path, *len, arr.len()));
            }
            write_seq(arr, |_| elem, doc, store, path, depth)
        }
        Node::Map { value: val, .. } => {
            // `validate_schema` has already checked every map, including
            // those in unused definitions and inactive variants.
            let obj = as_object(value, path)?;
            check_tree_len(obj.len(), path)?;
            let mut entries = Vec::with_capacity(obj.len());
            for (key, value) in obj.iter() {
                let key = key.as_str();
                let filename = escaped_name(key, path)?;
                let (oid, kind) = write_node(value, val, doc, store, &path.field(key), depth + 1)?;
                entries.push(TreeEntry {
                    mode: EntryMode::from(kind),
                    filename: filename.into(),
                    oid: oid.into_raw(),
                });
            }
            tree(store, entries, path)
        }
        Node::Optional(inner) => {
            if value.is_null() {
                // An unwrapped Value cannot distinguish None from Some(None)
                // or Some(()); null selects None.
                return marker_tree(store);
            }
            let (oid, kind) = write_node(
                value,
                inner,
                doc,
                store,
                &path.field(crate::marker::SOME_KEY),
                depth + 1,
            )?;
            tree(
                store,
                vec![TreeEntry {
                    mode: EntryMode::from(kind),
                    filename: crate::marker::SOME_KEY.into(),
                    oid: oid.into_raw(),
                }],
                path,
            )
        }
        Node::Enum(variants) => write_enum(value, variants, doc, store, path, depth),
        // A raw tree is opaque: the value is the referenced object id in hex,
        // handed straight back as a tree reference with no write — mirroring
        // the typed `RawTree` encoding, which likewise emits no object. The
        // parsed id must also use the store's hash format, so a wrong-format
        // reference is refused here rather than embedded.
        Node::RawTree => {
            let s = value_str(value).ok_or_else(|| expected(path, "object-id string", value))?;
            let invalid = || SchemaWriteError::InvalidRawTree {
                path: path.show(),
                text: s.to_owned(),
            };
            let raw = ObjectId::from_hex(s.as_bytes()).map_err(|_error| invalid())?;
            let oid = Oid::<F>::new(raw).map_err(|_error| invalid())?;
            Ok((oid, EntryKind::Tree))
        }
        // A dynamic node carries no schema, so the bare heuristic write — the
        // same one `serialize` applies to a `Value` field — is exactly right.
        Node::Dynamic => Ok(serialize_node(Peek::new(value), store, depth)?),
        Node::Ref(_) => {
            // References are transparent on disk: a chain of aliases is
            // equivalent to its target, and following one must not consume
            // tree-depth budget or recurse on the stack.
            let target = schema.resolve(doc.defs()).map_err(|error| match error {
                RefError::Dangling { name } => SchemaWriteError::UnknownRef {
                    path: path.show(),
                    name,
                },
                RefError::Cycle { name } => {
                    invalid_schema(&format!("non-productive ref cycle through {name:?}"))
                }
            })?;
            write_node(value, target, doc, store, path, depth)
        }
    }
}

/// Encode the required named fields of a struct or struct variant.
fn write_named_tree<F: HashFormat, W: Write + Tagged<F> + ?Sized>(
    value: &Value,
    fields: &BTreeMap<String, Node>,
    doc: &Schema,
    store: &W,
    path: &Path,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    check_depth(path, depth)?;
    let obj = as_object(value, path)?;
    check_tree_len(fields.len(), path)?;
    for k in obj.keys() {
        if !fields.contains_key(k.as_str()) {
            return Err(SchemaWriteError::UnknownField {
                path: path.show(),
                field: k.as_str().to_owned(),
            });
        }
    }
    let mut entries = Vec::with_capacity(fields.len());
    for (name, field) in fields {
        let filename = escaped_name(name, path)?;
        let field_value = obj
            .get(name.as_str())
            .ok_or_else(|| SchemaWriteError::MissingField {
                path: path.show(),
                field: name.clone(),
            })?;
        let (oid, kind) = write_node(field_value, field, doc, store, &path.field(name), depth + 1)?;
        entries.push(TreeEntry {
            mode: EntryMode::from(kind),
            filename: filename.into(),
            oid: oid.into_raw(),
        });
    }
    tree(store, entries, path)
}

/// Encode an array as a tree of contiguous three-digit hexadecimal ordinals.
fn write_seq<'s, F: HashFormat, W: Write + Tagged<F> + ?Sized>(
    arr: &VArray,
    schema_for: impl Fn(usize) -> &'s Node,
    doc: &Schema,
    store: &W,
    path: &Path,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    check_depth(path, depth)?;
    check_tree_len(arr.len(), path)?;
    let mut entries = Vec::with_capacity(arr.len());
    for (i, item) in arr.as_slice().iter().enumerate() {
        let filename = ordinal::encode(i)
            .map_err(|error| SerializeError::Reflect(format!("at {}: {error}", path.show())))?;
        let (oid, kind) = write_node(item, schema_for(i), doc, store, &path.index(i), depth + 1)?;
        entries.push(TreeEntry {
            mode: EntryMode::from(kind),
            filename: filename.into(),
            oid: oid.into_raw(),
        });
    }
    tree(store, entries, path)
}

/// Encode an enum: externally tagged by the live variant's name. A unit
/// variant collapses to a bare blob holding that name (its entire
/// information content); every other variant is a single-member object whose
/// value follows the variant's [`VariantKind`] layout, wrapped in a
/// single-entry tree keyed by the name.
fn write_enum<F: HashFormat, W: Write + Tagged<F> + ?Sized>(
    value: &Value,
    variants: &BTreeMap<String, VariantKind>,
    doc: &Schema,
    store: &W,
    path: &Path,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    let obj = as_object(value, path)?;
    if obj.len() != 1 {
        return Err(SchemaWriteError::MalformedEnum {
            path: path.show(),
            found: obj.len(),
        });
    }
    let Some((name, payload)) = obj.iter().next() else {
        return Err(SchemaWriteError::MalformedEnum {
            path: path.show(),
            found: 0,
        });
    };
    let name = name.as_str();
    let kind = variants
        .get(name)
        .ok_or_else(|| SchemaWriteError::UnknownVariant {
            path: path.show(),
            variant: name.to_owned(),
            expected: variants.keys().cloned().collect(),
        })?;
    let filename = escaped_name(name, path)?;
    let vpath = path.field(name);

    // A single tuple field is the payload itself; struct fields are always
    // an object, even when the struct has exactly one field.
    let (inner_oid, inner_kind) = match kind {
        VariantKind::Unit => {
            if !payload.is_null() {
                return Err(expected(&vpath, "null", payload));
            }
            return blob(store, filename.as_bytes());
        }
        VariantKind::Tuple(elems) => match elems.as_slice() {
            [inner] => write_node(payload, inner, doc, store, &vpath, depth + 1)?,
            _ => {
                let arr = as_array(payload, &vpath)?;
                if arr.len() != elems.len() {
                    return Err(length_mismatch(&vpath, elems.len(), arr.len()));
                }
                write_seq(arr, |i| &elems[i], doc, store, &vpath, depth + 1)?
            }
        },
        VariantKind::Struct(fields) => {
            write_named_tree(payload, fields, doc, store, &vpath, depth + 1)?
        }
    };
    tree(
        store,
        vec![TreeEntry {
            mode: EntryMode::from(inner_kind),
            filename: filename.into(),
            oid: inner_oid.into_raw(),
        }],
        path,
    )
}

// --- scalar helpers ---

fn float64(value: &Value, path: &Path) -> Result<f64, SchemaWriteError> {
    if let Some(text) = value_str(value) {
        return match text {
            "nan" => Ok(f64::NAN),
            "inf" => Ok(f64::INFINITY),
            "-inf" => Ok(f64::NEG_INFINITY),
            _ => Err(expected(path, "number or nan/inf/-inf", value)),
        };
    }
    let n = as_number(value, path)?;
    if n.is_float() {
        return Ok(n.to_f64_lossy());
    }
    if !integer_is_exact_float(n, f64::MANTISSA_DIGITS) {
        return Err(unrepresentable(path, "F64"));
    }
    Ok(n.to_f64_lossy())
}

fn float32(value: &Value, path: &Path) -> Result<f32, SchemaWriteError> {
    if let Some(text) = value_str(value) {
        return match text {
            "nan" => Ok(f32::NAN),
            "inf" => Ok(f32::INFINITY),
            "-inf" => Ok(f32::NEG_INFINITY),
            _ => Err(expected(path, "number or nan/inf/-inf", value)),
        };
    }
    let n = as_number(value, path)?;
    if !n.is_float() && !integer_is_exact_float(n, f32::MANTISSA_DIGITS) {
        return Err(unrepresentable(path, "F32"));
    }
    let source = n.to_f64_lossy();
    let result = source as f32;
    // Reject finite inputs that round to infinity, but allow those that round to f32::MAX.
    if source.is_finite() && result.is_infinite() {
        return Err(unrepresentable(path, "F32"));
    }
    Ok(result)
}

/// A binary float represents an integer exactly iff its nonzero significant
/// bits fit in the float's mantissa. Checking bits avoids saturating casts at
/// the signed and unsigned integer maxima. The f32 exponent range is checked
/// separately after converting.
fn integer_is_exact_float(n: &VNumber, mantissa_bits: u32) -> bool {
    let magnitude = if let Some(i) = n.to_i128() {
        i.unsigned_abs()
    } else if let Some(u) = n.to_u128() {
        u
    } else {
        return false;
    };
    magnitude == 0
        || u128::BITS - magnitude.leading_zeros() - magnitude.trailing_zeros() <= mantissa_bits
}

/// Signed integers of any width: the value must be an in-range integer (a
/// float-backed number is refused — the read path never emits one for an
/// integer node).
fn signed_blob<T, F, W>(
    value: &Value,
    schema: &'static str,
    path: &Path,
    store: &W,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError>
where
    T: TryFrom<i128> + core::fmt::Display,
    F: HashFormat,
    W: Write + ?Sized,
{
    let n = as_integer(value, schema, path)?;
    let i = n.to_i128().ok_or_else(|| out_of_range(path, schema, n))?;
    let t = T::try_from(i).map_err(|_| out_of_range(path, schema, n))?;
    blob(store, t.to_string().as_bytes())
}

/// Unsigned integers of any width. `to_u128` yields `None` for a negative
/// value, which is how a negative number is rejected for an unsigned node.
fn unsigned_blob<T, F, W>(
    value: &Value,
    schema: &'static str,
    path: &Path,
    store: &W,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError>
where
    T: TryFrom<u128> + core::fmt::Display,
    F: HashFormat,
    W: Write + ?Sized,
{
    let n = as_integer(value, schema, path)?;
    let u = n.to_u128().ok_or_else(|| out_of_range(path, schema, n))?;
    let t = T::try_from(u).map_err(|_| out_of_range(path, schema, n))?;
    blob(store, t.to_string().as_bytes())
}

// --- accessors and error builders ---

fn value_str(v: &Value) -> Option<&str> {
    v.as_string().map(|s| s.as_str())
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

fn as_number<'v>(value: &'v Value, path: &Path) -> Result<&'v VNumber, SchemaWriteError> {
    value
        .as_number()
        .ok_or_else(|| expected(path, "number", value))
}

/// A number that is integer-backed. A float-backed number is refused for an
/// integer schema node: the read path only ever emits integer-backed numbers
/// there, and float-to-integer is not one of the accepted JSON bridges.
fn as_integer<'v>(
    value: &'v Value,
    schema: &'static str,
    path: &Path,
) -> Result<&'v VNumber, SchemaWriteError> {
    let n = as_number(value, path)?;
    if n.is_float() {
        return Err(SchemaWriteError::Expected {
            path: path.show(),
            expected: schema,
            found: "float",
        });
    }
    Ok(n)
}

fn as_array<'v>(value: &'v Value, path: &Path) -> Result<&'v VArray, SchemaWriteError> {
    value
        .as_array()
        .ok_or_else(|| expected(path, "array", value))
}

fn as_object<'v>(
    value: &'v Value,
    path: &Path,
) -> Result<&'v facet_value::VObject, SchemaWriteError> {
    value
        .as_object()
        .ok_or_else(|| expected(path, "object", value))
}

fn blob<F: HashFormat, W: Write + ?Sized>(
    store: &W,
    bytes: &[u8],
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    let oid = write_leaf_blob(store, bytes)?;
    Ok((oid, EntryKind::Blob))
}

fn binary_blob<F: HashFormat, W: Write + ?Sized>(
    store: &W,
    bytes: &[u8],
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    let oid = write_binary_blob(store, bytes)?;
    Ok((oid, EntryKind::Blob))
}

fn tree<F: HashFormat, W: Write + ?Sized>(
    store: &W,
    mut entries: Vec<TreeEntry>,
    path: &Path,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    check_tree_len(entries.len(), path)?;
    if entries.is_empty() {
        return marker_tree(store);
    }
    // gix sorts using Git's implicit slash on tree entries, not plain names.
    entries.sort();
    let raw = store
        .write(&gix_object::Tree { entries })
        .map_err(SerializeError::Backend)?;
    let oid = wrap_oid(raw)?;
    Ok((oid, EntryKind::Tree))
}

/// Write the `none` marker instead of an empty tree.
fn marker_tree<F: HashFormat, W: Write + ?Sized>(
    store: &W,
) -> Result<(Oid<F>, EntryKind), SchemaWriteError> {
    let oid = crate::marker::write_marker_tree(store)?;
    Ok((oid, EntryKind::Tree))
}

fn check_depth(path: &Path, depth: usize) -> Result<(), SchemaWriteError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(SchemaWriteError::MaxDepth {
            path: path.show(),
            depth: MAX_VALUE_DEPTH,
        });
    }
    Ok(())
}

fn check_tree_len(len: usize, path: &Path) -> Result<(), SchemaWriteError> {
    if len > MAX_TREE_ENTRIES {
        return Err(SerializeError::Reflect(format!(
            "at {}: tree has {len} entries, exceeding the maximum of {MAX_TREE_ENTRIES}",
            path.show()
        ))
        .into());
    }
    Ok(())
}

fn escaped_name(name: &str, path: &Path) -> Result<String, SchemaWriteError> {
    crate::name::escape(name).map_err(|error| {
        SerializeError::Reflect(format!(
            "at {}: cannot encode name {name:?}: {error}",
            path.show()
        ))
        .into()
    })
}

fn invalid_schema(reason: &str) -> SchemaWriteError {
    SerializeError::Reflect(format!("invalid schema: {reason}")).into()
}

fn expected(path: &Path, expected: &'static str, found: &Value) -> SchemaWriteError {
    SchemaWriteError::Expected {
        path: path.show(),
        expected,
        found: value_kind(found),
    }
}

fn out_of_range(path: &Path, schema: &'static str, n: &VNumber) -> SchemaWriteError {
    SchemaWriteError::NumberOutOfRange {
        path: path.show(),
        schema,
        value: number_text(n),
    }
}

fn length_mismatch(path: &Path, expected: usize, found: usize) -> SchemaWriteError {
    SchemaWriteError::LengthMismatch {
        path: path.show(),
        expected,
        found,
    }
}

fn unrepresentable(path: &Path, schema: &'static str) -> SchemaWriteError {
    SchemaWriteError::UnrepresentableNumber {
        path: path.show(),
        schema,
    }
}

/// A number's value as text, for diagnostics only.
fn number_text(n: &VNumber) -> String {
    if let Some(i) = n.to_i128() {
        i.to_string()
    } else if let Some(u) = n.to_u128() {
        u.to_string()
    } else {
        n.to_f64_lossy().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectStore;
    use crate::schema::read::{deserialize_value_with_schema, validate_with_schema};
    use facet_value::{VObject, value};

    fn schema(root: Node) -> Schema {
        Schema::new("test".into(), root, BTreeMap::new()).expect("valid schema")
    }

    #[test]
    fn cbor_writes_reject_invalid_bytes_and_other_value_kinds() {
        let doc = schema(Node::Cbor);
        let doc = doc.checked().expect("valid schema");
        let store = ObjectStore::default();
        for (payload, reason) in [
            (vec![], "truncated"),
            (vec![0x00, 0x00], "trailing bytes"),
            (vec![0x00, 0x0a], "trailing bytes"),
            (vec![0x18, 0x17], "non-minimal"),
            (vec![0x5f, 0xff], "indefinite"),
            (vec![0x62, b'a'], "truncated"),
            (vec![0x61, 0xff], "UTF-8"),
            (vec![0xa2, 0x01, 0x00, 0x01, 0x01], "duplicate"),
            (vec![0xa2, 0x02, 0x00, 0x01, 0x01], "out-of-order"),
            (vec![0xfa, 0x3f, 0xc0, 0x00, 0x00], "non-canonical"),
        ] {
            let error =
                serialize_value_with_schema(&Value::from(payload), &doc, &store).unwrap_err();
            assert!(matches!(
                &error,
                SchemaWriteError::Serialize(SerializeError::Reflect(message))
                    if message.contains("at $") && message.contains(reason)
            ));
        }
        let mut too_deep = vec![0x81; MAX_VALUE_DEPTH + 1];
        too_deep.push(0xf6);
        assert!(serialize_value_with_schema(&Value::from(too_deep), &doc, &store).is_err());

        for input in [value!(""), value!("a"), value!(0), value!([1]), Value::NULL] {
            let error = serialize_value_with_schema(&input, &doc, &store).unwrap_err();
            assert!(matches!(
                error,
                SchemaWriteError::Expected { path, expected: "canonical CBOR bytes", .. }
                    if path == "$"
            ));
        }
    }

    #[test]
    fn cbor_nested_references_and_options_roundtrip() -> anyhow::Result<()> {
        let doc = Schema::new(
            "test".into(),
            Node::Ref("Root".into()),
            BTreeMap::from([
                (
                    "Root".into(),
                    Node::Struct(BTreeMap::from([
                        ("direct".into(), Node::Ref("Alias".into())),
                        (
                            "items".into(),
                            Node::List(Box::new(Node::Optional(Box::new(Node::Ref(
                                "Payload".into(),
                            ))))),
                        ),
                        (
                            "variant".into(),
                            Node::Enum(BTreeMap::from([(
                                "Payload".into(),
                                VariantKind::Tuple(vec![Node::Ref("Payload".into())]),
                            )])),
                        ),
                    ])),
                ),
                ("Alias".into(), Node::Ref("Payload".into())),
                ("Payload".into(), Node::Cbor),
            ]),
        )
        .expect("valid schema");
        let doc = doc.checked()?;
        let direct: &[u8] = &[0x0a];
        let cbor_null: &[u8] = &[0xf6];
        let cbor_bytes: &[u8] = &[0x42, 0x00, 0xff];
        let mut variant = VObject::new();
        variant.insert("Payload", Value::from(cbor_bytes));
        let mut items = VArray::new();
        items.push(Value::from(cbor_null));
        items.push(Value::NULL);
        items.push(Value::from(cbor_bytes));
        let mut fields = VObject::new();
        fields.insert("direct", Value::from(direct));
        fields.insert("items", items);
        fields.insert("variant", variant);
        let value: Value = fields.into();
        let store = ObjectStore::default();
        let root = serialize_value_with_schema(&value, &doc, &store)?;
        let root_entries = store.get_tree(&root)?.expect("root tree");
        let direct_oid = root_entries
            .iter()
            .find(|entry| entry.filename == "direct")
            .expect("direct field")
            .oid;
        let direct_oid = Oid::<crate::hash::Sha1>::new(direct_oid).expect("direct id is sha1");
        assert_eq!(store.get_blob(&direct_oid)?.as_deref(), Some(direct));
        validate_with_schema(&root, &doc, &store)?;
        let decoded = deserialize_value_with_schema(&root, &doc, &store)?;
        assert_eq!(decoded, value);
        let decoded_items = decoded
            .as_object()
            .unwrap()
            .get("items")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            decoded_items.as_slice()[0].as_bytes().unwrap().as_slice(),
            cbor_null
        );
        assert!(decoded_items.as_slice()[1].is_null());
        let output = ObjectStore::default();
        assert_eq!(serialize_value_with_schema(&decoded, &doc, &output)?, root);

        let mut invalid = VObject::new();
        invalid.insert("direct", Value::from(direct));
        let mut invalid_items = VArray::new();
        invalid_items.push(Value::from(vec![0x00, 0x00]));
        invalid.insert("items", invalid_items);
        invalid.insert("variant", value!({ "Payload": null }));
        let error = serialize_value_with_schema(&invalid.into(), &doc, &store).unwrap_err();
        assert!(matches!(
            error,
            SchemaWriteError::Serialize(SerializeError::Reflect(message))
                if message.contains("$.items[0].some") && message.contains("trailing")
        ));
        Ok(())
    }

    #[test]
    fn cbor_keeps_full_schema_validation_before_value_walk() {
        let defs = |def: Node| BTreeMap::from([("unused".to_owned(), def)]);

        // A dangling reference in an unused definition is rejected when the
        // document is validated, not lazily when a value is walked.
        assert!(matches!(
            Schema::new("test".into(), Node::Cbor, defs(Node::Ref("missing".into()))),
            Err(crate::error::SchemaReadError::UnknownRef(name)) if name == "missing"
        ));

        // An unguarded self-reference and a non-`String` map key are likewise
        // rejected up front, before any value write can be attempted.
        assert!(Schema::new("test".into(), Node::Cbor, defs(Node::Ref("unused".into()))).is_err());
        assert!(
            Schema::new(
                "test".into(),
                Node::Cbor,
                defs(Node::Map {
                    key: Box::new(Node::Cbor),
                    value: Box::new(Node::Cbor),
                }),
            )
            .is_err()
        );
    }
}
