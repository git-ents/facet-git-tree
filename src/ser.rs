//! Serialization: encoding [`facet::Facet`] values as Git objects.

use facet::{Def, DynDateTimeKind, DynValueKind, Peek, ScalarType};
use gix_object::{Kind, Write};

use std::collections::BTreeSet;

use crate::attr;
use crate::classify::{ShapeClass, classify, collapse_shape};
use crate::error::SerializeError;
use crate::hash::{HashFormat, Oid, Sha1, Tagged};
use crate::limits::MAX_VALUE_DEPTH;
use crate::ordinal::{self, MAX_TREE_ENTRIES};
use crate::store::ObjectStore;
use crate::{EntryKind, EntryMode, ObjectId, RawBlob, RawTree, TreeEntry};

/// Collapse a `facet` reflection error to [`SerializeError::Reflect`]
/// (reflection errors are not `'static`-friendly).
fn reflect(e: impl std::fmt::Display) -> SerializeError {
    SerializeError::Reflect(e.to_string())
}

/// Wrap a freshly written object id as an [`Oid<F>`].
///
/// A backend that honors the store's claimed format always produces a
/// matching id, so a mismatch here is a backend bug; report it rather than
/// panicking.
pub(crate) fn wrap_oid<F: HashFormat>(raw: ObjectId) -> Result<Oid<F>, SerializeError> {
    Oid::new(raw).map_err(|error| {
        reflect(format!(
            "backend produced an object id of the wrong hash format: {error}"
        ))
    })
}

fn escaped_name(name: &str) -> Result<String, SerializeError> {
    crate::name::escape(name)
        .map_err(|error| reflect(format!("cannot encode name {name:?}: {error}")))
}

fn check_tree_len(len: usize) -> Result<(), SerializeError> {
    if len > MAX_TREE_ENTRIES {
        return Err(reflect(format!(
            "tree has {len} entries, exceeding the maximum of {MAX_TREE_ENTRIES}"
        )));
    }
    Ok(())
}

fn check_depth(depth: usize) -> Result<(), SerializeError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(SerializeError::MaxDepth(MAX_VALUE_DEPTH));
    }
    Ok(())
}

/// Serialize a [`facet::Facet`] value into the given `gix` object `store`.
///
/// The hash format `F` is inferred from the store: a concrete
/// [`ObjectStore<F>`](crate::ObjectStore) or a
/// [`Backend<F, _>`](crate::Backend) wrapper both claim their format through
/// [`Tagged`]. Returns the bare root object's [`Oid<F>`]; scalar roots can be
/// blobs.
pub fn serialize_into<T, F, W>(value: &T, store: &W) -> Result<Oid<F>, SerializeError>
where
    T: for<'a> facet::Facet<'a>,
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    serialize_root(Peek::new(value), store)
}

/// Serialize a [`facet::Facet`] value into a fresh [`ObjectStore`].
///
/// Dynamic values have no runtime type markers, so their heuristic read is
/// intentionally lossy; typed values round-trip through their supplied shape.
///
/// Returns the bare root object's [`Oid`] and the store containing its
/// reachable objects, both in SHA-1.
pub fn serialize<T: for<'a> facet::Facet<'a>>(
    value: &T,
) -> Result<(Oid<Sha1>, ObjectStore<Sha1>), SerializeError> {
    let store = ObjectStore::default();
    let root = serialize_root(Peek::new(value), &store)?;
    Ok((root, store))
}

fn serialize_root<F, W>(peek: Peek<'_, '_>, store: &W) -> Result<Oid<F>, SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let (oid, _kind) = serialize_node(peek, store, 0)?;
    Ok(oid)
}

pub(crate) fn serialize_node<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    check_depth(depth)?;
    let peek = peek.innermost_peek();
    let shape = peek.shape();

    if shape.scalar_type() == Some(ScalarType::Unit) {
        return Ok((crate::marker::write_marker_tree(store)?, EntryKind::Tree));
    }

    match classify(shape) {
        ShapeClass::RawTree => {
            // The shape is `RawTree<F'>`; requesting `RawTree<F>` fails for a
            // tree of the other format, so a mismatched passthrough is refused.
            let rt = peek.get::<RawTree<F>>().map_err(reflect)?;
            Ok((rt.oid(), EntryKind::Tree))
        }
        ShapeClass::RawBlob => {
            let rb = peek.get::<RawBlob<F>>().map_err(reflect)?;
            Ok((rb.oid(), EntryKind::Blob))
        }
        ShapeClass::Dynamic => serialize_dynamic(peek, store, depth),
        ShapeClass::Scalar => serialize_leaf(peek, store),
        ShapeClass::Bytes => serialize_byte_sequence(peek, store),
        ShapeClass::Struct => serialize_struct(peek, store, depth),
        ShapeClass::Sequence => serialize_sequence(peek, store, depth),
        ShapeClass::Map => serialize_map(peek, store, depth),
        ShapeClass::Option => serialize_option(peek, store, depth),
        ShapeClass::Enum => serialize_enum(peek, store, depth),
        ShapeClass::TransparentPointer | ShapeClass::TransparentNewtype => {
            Err(SerializeError::Unsupported(shape.type_identifier))
        }
        ShapeClass::Unsupported => Err(SerializeError::Unsupported(shape.type_identifier)),
    }
}

fn serialize_leaf<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let oid = write_leaf_blob(store, &scalar_bytes(peek)?)?;
    Ok((oid, EntryKind::Blob))
}

fn serialize_byte_sequence<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    // Bulk path: a contiguous `u8` list-like (Vec, array, slice) hands over
    // its whole buffer in one call instead of one reflection call per byte —
    // the per-byte loop below is the fallback for non-contiguous element
    // storage.
    let seq = peek.into_list_like().map_err(reflect)?;
    if let Some(bytes) = seq.as_bytes() {
        let oid = write_binary_blob(store, bytes)?;
        return Ok((oid, EntryKind::Blob));
    }
    let mut bytes = Vec::new();
    for item in seq.iter() {
        bytes.push(*item.get::<u8>().map_err(reflect)?);
    }
    let oid = write_binary_blob(store, &bytes)?;
    Ok((oid, EntryKind::Blob))
}

fn serialize_struct<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let facet::Type::User(facet::UserType::Struct(st)) = peek.shape().ty else {
        return Err(SerializeError::Unsupported(peek.shape().type_identifier));
    };
    check_tree_len(st.fields.len())?;
    let positional = matches!(
        st.kind,
        facet::StructKind::Tuple | facet::StructKind::TupleStruct
    );
    let ps = peek.into_struct().map_err(reflect)?;
    let mut entries = Vec::with_capacity(st.fields.len());
    for (i, field) in st.fields.iter().enumerate() {
        let filename = if positional {
            ordinal::encode(i).map_err(reflect)?
        } else {
            escaped_name(field.effective_name())?
        };
        let (oid, kind) = serialize_node(ps.field(i).map_err(reflect)?, store, depth + 1)?;
        entries.push(TreeEntry {
            mode: EntryMode::from(kind),
            filename: filename.into(),
            oid: oid.into_raw(),
        });
    }
    Ok((write_tree_or_marker(store, entries)?, EntryKind::Tree))
}

/// Serialize a sequence as an ordinal-named tree, or the marker if empty.
fn serialize_sequence<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let seq = peek.into_list_like().map_err(reflect)?;
    check_tree_len(seq.len())?;
    let mut entries = Vec::with_capacity(seq.len());
    for (i, item) in seq.iter().enumerate() {
        let filename = ordinal::encode(i).map_err(reflect)?;
        let (oid, kind) = serialize_node(item, store, depth + 1)?;
        entries.push(TreeEntry {
            mode: EntryMode::from(kind),
            filename: filename.into(),
            oid: oid.into_raw(),
        });
    }
    Ok((write_tree_or_marker(store, entries)?, EntryKind::Tree))
}

/// Serialize string-keyed maps as trees and other maps as canonical CBOR blobs.
fn serialize_map<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let shape = peek.shape();
    let Def::Map(md) = shape.def else {
        return Err(SerializeError::Unsupported(shape.type_identifier));
    };
    let key_shape = collapse_shape(md.k);
    if attr::is_identity_key(md.k.attributes)
        || !matches!(
            key_shape.scalar_type(),
            Some(ScalarType::Str | ScalarType::String | ScalarType::CowStr)
        )
    {
        let bytes = crate::cbor::encode_peek(peek).map_err(|error| {
            reflect(format!(
                "cannot encode {} as CBOR: {error}",
                shape.type_identifier
            ))
        })?;
        return Ok((write_binary_blob(store, &bytes)?, EntryKind::Blob));
    }
    let pm = peek.into_map().map_err(reflect)?;
    check_tree_len(pm.len())?;
    let mut entries = Vec::with_capacity(pm.len());
    for (key, value) in pm.iter() {
        let key = key
            .as_str()
            .ok_or_else(|| reflect("string map key unreadable"))?;
        let filename = escaped_name(key)?;
        let (oid, kind) = serialize_node(value, store, depth + 1)?;
        entries.push(TreeEntry {
            mode: EntryMode::from(kind),
            filename: filename.into(),
            oid: oid.into_raw(),
        });
    }
    Ok((write_tree_or_marker(store, entries)?, EntryKind::Tree))
}

fn serialize_option<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let po = peek.into_option().map_err(reflect)?;
    let Some(inner) = po.value() else {
        return Ok((write_tree_or_marker(store, Vec::new())?, EntryKind::Tree));
    };
    let (oid, kind) = serialize_node(inner, store, depth + 1)?;
    let entries = vec![TreeEntry {
        mode: EntryMode::from(kind),
        filename: crate::marker::SOME_KEY.into(),
        oid: oid.into_raw(),
    }];
    Ok((write_sorted_tree(store, entries)?, EntryKind::Tree))
}

fn serialize_enum<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let pe = peek.into_enum().map_err(reflect)?;
    let variant = pe.active_variant().map_err(reflect)?;
    let variant_name = escaped_name(variant.effective_name())?;
    if matches!(variant.data.kind, facet::StructKind::Unit) {
        return Ok((
            write_leaf_blob(store, variant_name.as_bytes())?,
            EntryKind::Blob,
        ));
    }

    let fields = variant.data.fields;
    check_tree_len(fields.len())?;
    let positional = matches!(
        variant.data.kind,
        facet::StructKind::Tuple | facet::StructKind::TupleStruct
    );
    let (inner_oid, inner_kind) = if positional && fields.len() == 1 {
        let child = pe
            .field(0)
            .map_err(reflect)?
            .ok_or_else(|| reflect("variant field 0 missing"))?;
        serialize_node(child, store, depth + 1)?
    } else {
        // The payload itself is a tree nested under the tag tree.
        check_depth(depth + 1)?;
        let mut entries = Vec::with_capacity(fields.len());
        for (i, field) in fields.iter().enumerate() {
            let filename = if positional {
                ordinal::encode(i).map_err(reflect)?
            } else {
                escaped_name(field.effective_name())?
            };
            let child = pe
                .field(i)
                .map_err(reflect)?
                .ok_or_else(|| reflect(format!("variant field {i} missing")))?;
            let (oid, kind) = serialize_node(child, store, depth + 2)?;
            entries.push(TreeEntry {
                mode: EntryMode::from(kind),
                filename: filename.into(),
                oid: oid.into_raw(),
            });
        }
        (write_tree_or_marker(store, entries)?, EntryKind::Tree)
    };
    let entries = vec![TreeEntry {
        mode: EntryMode::from(inner_kind),
        filename: variant_name.into(),
        oid: inner_oid.into_raw(),
    }];
    Ok((write_sorted_tree(store, entries)?, EntryKind::Tree))
}

fn write_sorted_tree<F, W>(store: &W, mut entries: Vec<TreeEntry>) -> Result<Oid<F>, SerializeError>
where
    F: HashFormat,
    W: Write + ?Sized,
{
    check_tree_len(entries.len())?;
    if entries.is_empty() {
        return Err(reflect("cannot write a literal empty tree"));
    }
    let mut names = BTreeSet::new();
    for entry in &entries {
        if !names.insert(&entry.filename) {
            return Err(reflect(format!(
                "duplicate encoded tree entry name {:?}",
                entry.filename
            )));
        }
    }
    // `Entry::Ord` is gix's Git comparator, including the implicit slash for
    // directories. A bytewise name sort is not equivalent for prefix names.
    entries.sort();
    wrap_oid(
        store
            .write(&gix_object::Tree { entries })
            .map_err(SerializeError::Backend)?,
    )
}

fn write_tree_or_marker<F, W>(store: &W, entries: Vec<TreeEntry>) -> Result<Oid<F>, SerializeError>
where
    F: HashFormat,
    W: Write + ?Sized,
{
    if entries.is_empty() {
        crate::marker::write_marker_tree(store)
    } else {
        write_sorted_tree(store, entries)
    }
}

/// Serialize a dynamic value (`Def::DynamicValue`) by dispatching on its
/// runtime kind.
///
/// Dynamic kinds carry no on-disk tag. Values whose textual representation
/// cannot be recovered exactly from the runtime interface are rejected.
fn serialize_dynamic<F, W>(
    peek: Peek<'_, '_>,
    store: &W,
    depth: usize,
) -> Result<(Oid<F>, EntryKind), SerializeError>
where
    F: HashFormat,
    W: Write + Tagged<F> + ?Sized,
{
    let dv = peek.into_dynamic_value().map_err(reflect)?;

    let blob = |bytes: &[u8]| -> Result<(Oid<F>, EntryKind), SerializeError> {
        let oid = write_leaf_blob(store, bytes)?;
        Ok((oid, EntryKind::Blob))
    };
    let tree_or_marker = |entries: Vec<TreeEntry>| -> Result<(Oid<F>, EntryKind), SerializeError> {
        Ok((write_tree_or_marker(store, entries)?, EntryKind::Tree))
    };

    match dv.kind() {
        DynValueKind::Null => tree_or_marker(vec![]),
        DynValueKind::Bool => {
            let b = dv
                .as_bool()
                .ok_or_else(|| reflect("dynamic bool unreadable"))?;
            blob(if b { "true" } else { "false" }.as_bytes())
        }
        // Strings (and dynamic chars, which surface through this same arm
        // as their UTF-8 representation rather than a kind of their own).
        DynValueKind::String => {
            let s = dv
                .as_str()
                .ok_or_else(|| reflect("dynamic string unreadable"))?;
            blob(s.as_bytes())
        }
        DynValueKind::Bytes => {
            let b = dv
                .as_bytes()
                .ok_or_else(|| reflect("dynamic bytes unreadable"))?;
            // The dynamic-kind rule specifies a text leaf, even for bytes.
            blob(b)
        }
        DynValueKind::Number => {
            // Inspect facet-value's actual storage before the numeric accessors:
            // its integer accessors also accept integral floats and would
            // otherwise turn `1.0` into the integer spelling `1`.
            if peek.shape().is_type::<facet_value::Value>() {
                let value = peek.get::<facet_value::Value>().map_err(reflect)?;
                let number = value
                    .as_number()
                    .ok_or_else(|| reflect("dynamic number unreadable"))?;
                if number.is_float() {
                    return blob(crate::float::format_f64(number.to_f64_lossy()).as_bytes());
                }
                if let Some(i) = number.to_i128() {
                    return blob(i.to_string().as_bytes());
                }
                if let Some(u) = number.to_u128() {
                    return blob(u.to_string().as_bytes());
                }
                return Err(SerializeError::UnrepresentableNumber);
            }
            if let Some(i) = dv.as_i64() {
                return blob(i.to_string().as_bytes());
            }
            if let Some(u) = dv.as_u64() {
                return blob(u.to_string().as_bytes());
            }
            if let Some(f) = dv.as_f64() {
                // A whole value here may be a lossy image of an out-of-range
                // integer; reject it rather than change the object id.
                if f.is_finite() && f.trunc() == f {
                    return Err(SerializeError::UnrepresentableNumber);
                }
                return blob(crate::float::format_f64(f).as_bytes());
            }
            Err(SerializeError::UnrepresentableNumber)
        }
        DynValueKind::Array => {
            let iter = dv
                .array_iter()
                .ok_or_else(|| reflect("dynamic array unreadable"))?;
            check_tree_len(iter.len())?;
            let mut entries = Vec::with_capacity(iter.len());
            for (i, item) in iter.enumerate() {
                let filename = ordinal::encode(i).map_err(reflect)?;
                let (oid, kind) = serialize_node(item, store, depth + 1)?;
                entries.push(TreeEntry {
                    mode: EntryMode::from(kind),
                    filename: filename.into(),
                    oid: oid.into_raw(),
                });
            }
            tree_or_marker(entries)
        }
        DynValueKind::Object => {
            let iter = dv
                .object_iter()
                .ok_or_else(|| reflect("dynamic object unreadable"))?;
            check_tree_len(iter.len())?;
            let mut entries = Vec::with_capacity(iter.len());
            for (key, value) in iter {
                let filename = escaped_name(key)?;
                let (oid, kind) = serialize_node(value, store, depth + 1)?;
                entries.push(TreeEntry {
                    mode: EntryMode::from(kind),
                    filename: filename.into(),
                    oid: oid.into_raw(),
                });
            }
            tree_or_marker(entries)
        }
        DynValueKind::DateTime => {
            let parts = dv
                .as_datetime()
                .ok_or_else(|| reflect("dynamic datetime unreadable"))?;
            blob(datetime_text(parts)?.as_bytes())
        }
        // The dynamic vtable exposes no QName/UUID accessors. The concrete
        // facet-value downcast is available on every feature combination.
        kind @ (DynValueKind::QName | DynValueKind::Uuid) => {
            if let Some(text) = value_special_text(peek)? {
                return blob(text.as_bytes());
            }
            Err(SerializeError::UnsupportedDynamicKind(format!("{kind:?}")))
        }
        // Refuse future kinds rather than guess an encoding.
        other => Err(SerializeError::UnsupportedDynamicKind(format!("{other:?}"))),
    }
}

/// Render a dynamic datetime as RFC 3339-style text; negative years pad the
/// magnitude separately (`-5` becomes `-0005`), years `10000`+ are not
/// truncated.
#[allow(clippy::type_complexity)] // the vtable's datetime tuple, taken as-is
fn datetime_text(
    parts: (i32, u8, u8, u8, u8, u8, u32, DynDateTimeKind),
) -> Result<String, SerializeError> {
    let (year, month, day, hour, minute, second, nanos, kind) = parts;
    let year_text = if year < 0 {
        format!("-{:04}", year.unsigned_abs())
    } else {
        format!("{year:04}")
    };
    let date = format!("{year_text}-{month:02}-{day:02}");
    let mut time = format!("{hour:02}:{minute:02}:{second:02}");
    if nanos > 0 {
        let mut frac = format!("{nanos:09}");
        while frac.ends_with('0') {
            frac.pop();
        }
        time.push('.');
        time.push_str(&frac);
    }
    Ok(match kind {
        DynDateTimeKind::Offset { offset_minutes: 0 } => format!("{date}T{time}Z"),
        DynDateTimeKind::Offset { offset_minutes } => {
            let sign = if offset_minutes < 0 { '-' } else { '+' };
            let mag = offset_minutes.unsigned_abs();
            format!("{date}T{time}{sign}{:02}:{:02}", mag / 60, mag % 60)
        }
        DynDateTimeKind::LocalDateTime => format!("{date}T{time}"),
        DynDateTimeKind::LocalDate => date,
        DynDateTimeKind::LocalTime => time,
        // Refuse future kinds rather than emit ambiguous text.
        other => {
            return Err(SerializeError::UnsupportedDynamicKind(format!(
                "DateTime({other:?})"
            )));
        }
    })
}

/// The textual form of a `facet_value::Value` QName or UUID, or `None` when
/// `peek` is not a `facet_value::Value` holding one of those kinds.
///
/// UUIDs use canonical lowercase hyphenated hex; QNames use Clark notation
/// only for non-empty namespaces. Empty and absent namespaces therefore hash
/// to the same blob.
fn value_special_text(peek: Peek<'_, '_>) -> Result<Option<String>, SerializeError> {
    use std::fmt::Write as _;

    if !peek.shape().is_type::<facet_value::Value>() {
        return Ok(None);
    }
    let v = peek.get::<facet_value::Value>().map_err(reflect)?;
    if let Some(u) = v.as_uuid() {
        let mut s = String::with_capacity(36);
        for (i, byte) in u.as_bytes().iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                s.push('-');
            }
            write!(s, "{byte:02x}")
                .map_err(|error| reflect(format!("formatting UUID failed: {error}")))?;
        }
        return Ok(Some(s));
    }
    if let Some(q) = v.as_qname() {
        let local = q
            .local_name()
            .as_string()
            .ok_or_else(|| reflect("qname local name is not a string"))?
            .as_str();
        let namespace = match q.namespace() {
            Some(ns) => Some(
                ns.as_string()
                    .ok_or_else(|| reflect("qname namespace is not a string"))?
                    .as_str(),
            ),
            None => None,
        };
        return Ok(Some(match namespace {
            Some(ns) if !ns.is_empty() => format!("{{{ns}}}{local}"),
            _ => local.to_owned(),
        }));
    }
    Ok(None)
}

/// Write `content` as a leaf blob with exactly one trailing `\n`, so
/// content ending in `\n` stays distinguishable from content that does not.
pub(crate) fn write_leaf_blob<F, W>(store: &W, content: &[u8]) -> Result<Oid<F>, SerializeError>
where
    F: HashFormat,
    W: Write + ?Sized,
{
    let mut bytes = Vec::with_capacity(content.len() + 1);
    bytes.extend_from_slice(content);
    bytes.push(b'\n');
    wrap_oid(
        store
            .write_buf(Kind::Blob, &bytes)
            .map_err(SerializeError::Backend)?,
    )
}

/// Write a byte or CBOR leaf without a trailing newline.
pub(crate) fn write_binary_blob<F, W>(store: &W, content: &[u8]) -> Result<Oid<F>, SerializeError>
where
    F: HashFormat,
    W: Write + ?Sized,
{
    wrap_oid(
        store
            .write_buf(Kind::Blob, content)
            .map_err(SerializeError::Backend)?,
    )
}

fn scalar_bytes(peek: Peek<'_, '_>) -> Result<Vec<u8>, SerializeError> {
    let shape = peek.shape();
    let scalar = shape.scalar_type();
    let text = match scalar {
        Some(ScalarType::Str | ScalarType::String | ScalarType::CowStr) => {
            return peek
                .as_str()
                .map(|text| text.as_bytes().to_vec())
                .ok_or_else(|| reflect("string scalar unreadable"));
        }
        Some(ScalarType::Bool) => peek.get::<bool>().map_err(reflect)?.to_string(),
        Some(ScalarType::Char) => peek.get::<char>().map_err(reflect)?.to_string(),
        Some(ScalarType::F32) => crate::float::format_f32(*peek.get::<f32>().map_err(reflect)?),
        Some(ScalarType::F64) => crate::float::format_f64(*peek.get::<f64>().map_err(reflect)?),
        Some(ScalarType::U8) => peek.get::<u8>().map_err(reflect)?.to_string(),
        Some(ScalarType::U16) => peek.get::<u16>().map_err(reflect)?.to_string(),
        Some(ScalarType::U32) => peek.get::<u32>().map_err(reflect)?.to_string(),
        Some(ScalarType::U64) => peek.get::<u64>().map_err(reflect)?.to_string(),
        Some(ScalarType::U128) => peek.get::<u128>().map_err(reflect)?.to_string(),
        Some(ScalarType::I8) => peek.get::<i8>().map_err(reflect)?.to_string(),
        Some(ScalarType::I16) => peek.get::<i16>().map_err(reflect)?.to_string(),
        Some(ScalarType::I32) => peek.get::<i32>().map_err(reflect)?.to_string(),
        Some(ScalarType::I64) => peek.get::<i64>().map_err(reflect)?.to_string(),
        Some(ScalarType::I128) => peek.get::<i128>().map_err(reflect)?.to_string(),
        // Platform-width values have the same decimal representation as the
        // corresponding fixed-width values and are checked on wider targets.
        Some(ScalarType::USize) => u64::try_from(*peek.get::<usize>().map_err(reflect)?)
            .map_err(|_| SerializeError::UnsupportedScalar(shape.type_identifier))?
            .to_string(),
        Some(ScalarType::ISize) => i64::try_from(*peek.get::<isize>().map_err(reflect)?)
            .map_err(|_| SerializeError::UnsupportedScalar(shape.type_identifier))?
            .to_string(),
        _ => return Err(SerializeError::UnsupportedScalar(shape.type_identifier)),
    };
    Ok(text.into_bytes())
}
