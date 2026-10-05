//! Deserialization: decoding Git trees back into [`facet::Facet`] values.
//!
//! Typed targets round-trip faithfully; schemaless dynamic-value reads are a
//! documented lossy heuristic.

use facet::{Def, Partial};
use facet_value::Value;
use gix_hash::Kind as HashKind;
use gix_object::{Data, Find, Kind};
use std::collections::BTreeSet;

pub(crate) use crate::classify::collapse_shape;
use crate::classify::{ShapeClass, classify, is_byte_seq};
use crate::error::{DeserializeError, KeyError};
use crate::limits::MAX_VALUE_DEPTH;
use crate::{EntryKind, ObjectId, RawBlob, RawTree};

/// Collapse a `facet` reflection error to [`DeserializeError::Reflect`].
///
/// `facet`'s `Partial` operations return their own non-`'static` error types;
/// this collapses them to the dedicated text-carrying variant at the call site
/// without a bespoke closure every time.
fn reflect(e: impl std::fmt::Display) -> DeserializeError {
    DeserializeError::Reflect(e.to_string())
}

/// Whether a decoder accepts the historical leaf-blob spelling.
///
/// `Strict` is the ordinary format and requires the mandatory trailing newline.
/// `LegacyLeaves` is an explicit compatibility mode for reading pre-newline
/// objects; it never changes what the serializer writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeMode {
    /// Read only the current leaf-blob format.
    Strict,
    /// Also accept leaf blobs that predate the trailing newline.
    LegacyLeaves,
}

/// Validate a user-supplied key for use as a Git tree entry name.
///
/// NUL is checked here rather than left to the object backend, which can
/// only reject such a name as an opaque encode-time error while the name
/// is still in hand as user data.
pub fn check_key(key: &str) -> Result<(), KeyError> {
    if !is_tree_entry_name(key) {
        return Err(KeyError {
            key: key.to_owned(),
        });
    }
    Ok(())
}

/// Whether `name` is usable as a Git tree entry name.
///
/// The one statement of the rules shared by every name-to-entry site
/// ([`check_key`], [`crate::normal_form::Key::name`], and the identity
/// normal form's struct fields): non-empty; not `.` or `..`; no `/`; no NUL;
/// not the reserved marker name.
pub(crate) fn is_tree_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
        && name != crate::marker::MARKER_KEY
}

/// Deserialize a [`facet::Facet`] value from a root tree stored in `store`.
///
/// Typed targets round-trip faithfully; a schemaless dynamic-value target is
/// intentionally lossy (see `deser_dynamic` in this module).
///
/// `store` is any `gix` [`Find`] source; `?Sized` permits a `&dyn Find`.
pub fn deserialize<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
) -> Result<T, DeserializeError> {
    deserialize_at_depth(root, store, 0)
}

/// Decode a value while accepting pre-newline leaf blobs.
///
/// Separate from [`deserialize`]: callers must opt into compatibility with
/// the historical object spelling explicitly.
pub fn deserialize_legacy_leaves<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
) -> Result<T, DeserializeError> {
    deserialize_at_depth_mode(root, store, 0, DecodeMode::LegacyLeaves)
}

/// [`deserialize`], but with the recursion-depth budget starting at `depth`
/// instead of `0`, so a caller already inside a larger deserialization
/// (schema-driven reads route back through here) continues the same budget.
pub(crate) fn deserialize_at_depth<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
    depth: usize,
) -> Result<T, DeserializeError> {
    deserialize_at_depth_mode(root, store, depth, DecodeMode::Strict)
}

pub(crate) fn deserialize_at_depth_mode<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
    depth: usize,
    mode: DecodeMode,
) -> Result<T, DeserializeError> {
    let partial = Partial::alloc::<T>()
        .map_err(|e| DeserializeError::Reflect(format!("alloc failed: {e}")))?;
    let partial = deser_into(partial, root, store, depth, mode)?;
    let heap = partial
        .build()
        .map_err(|e| DeserializeError::Reflect(format!("build failed: {e}")))?;
    heap.materialize::<T>()
        .map_err(|e| DeserializeError::Reflect(format!("materialize failed: {e}")))
}

/// Deserialize the tree at `root` into an existing [`Partial`], mirroring
/// `facet`'s `*_into` convention: the caller owns allocation and `build`.
pub fn deserialize_into<'facet>(
    partial: Partial<'facet, true>,
    root: &ObjectId,
    store: &(impl Find + ?Sized),
) -> Result<Partial<'facet, true>, DeserializeError> {
    deser_into(partial, root, store, 0, DecodeMode::Strict)
}

pub(crate) fn find_object<'a, F: Find + ?Sized>(
    id: &ObjectId,
    buf: &'a mut Vec<u8>,
    store: &F,
) -> Result<Data<'a>, DeserializeError> {
    store
        .try_find(id, buf)
        .map_err(DeserializeError::Backend)?
        .ok_or_else(|| DeserializeError::NotFound(*id))
}

/// Parse an already-fetched object's [`Data`] as tree entries, without
/// re-fetching.
pub(crate) fn tree_entries_from_data(
    data: &Data<'_>,
    id: &ObjectId,
) -> Result<Vec<(String, ObjectId, EntryKind)>, DeserializeError> {
    if data.kind != Kind::Tree {
        return Err(DeserializeError::NotATree(*id));
    }
    let tree_ref = gix_object::TreeRef::from_bytes(data.data, HashKind::Sha1)
        .map_err(|source| DeserializeError::Decode { oid: *id, source })?;
    let mut result = Vec::new();
    for entry in &tree_ref.entries {
        let name = std::str::from_utf8(entry.filename).map_err(|_| {
            DeserializeError::NonUtf8Name(String::from_utf8_lossy(entry.filename).into_owned())
        })?;
        result.push((name.to_owned(), entry.oid.to_owned(), entry.mode.kind()));
    }
    Ok(result)
}

pub(crate) fn find_tree_entries<F: Find + ?Sized>(
    id: &ObjectId,
    store: &F,
) -> Result<Vec<(String, ObjectId, EntryKind)>, DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object(id, &mut buf, store)?;
    tree_entries_from_data(&data, id)
}

pub(crate) fn find_blob_bytes_mode<F: Find + ?Sized>(
    id: &ObjectId,
    store: &F,
    mode: DecodeMode,
) -> Result<Vec<u8>, DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object(id, &mut buf, store)?;
    if data.kind != Kind::Blob {
        return Err(DeserializeError::NotABlob(*id));
    }
    strip_leaf_newline(id, data.data.to_owned(), mode)
}

/// Strip a leaf blob's trailing newline according to `mode`; strict mode
/// reports absence as [`DeserializeError::MissingLeafNewline`], legacy mode
/// also accepts the historical no-newline spelling.
pub(crate) fn strip_leaf_newline(
    id: &ObjectId,
    mut bytes: Vec<u8>,
    mode: DecodeMode,
) -> Result<Vec<u8>, DeserializeError> {
    // Both modes strip the mandatory trailing newline; legacy mode
    // additionally accepts its historical absence.
    match (mode, bytes.last().copied()) {
        (DecodeMode::Strict | DecodeMode::LegacyLeaves, Some(b'\n')) => {
            bytes.pop();
            Ok(bytes)
        }
        (DecodeMode::LegacyLeaves, Some(_)) => Ok(bytes),
        (_, _) => Err(DeserializeError::MissingLeafNewline(*id)),
    }
}

/// Sort sequence entries into ascending ordinal order, rejecting names that
/// are not decimal indices or that repeat another entry's index (e.g. `"0"`
/// and `"0000"`).
pub(crate) fn sort_by_ordinal(
    entries: &mut [(String, ObjectId, EntryKind)],
) -> Result<(), DeserializeError> {
    // Sequence entry names are validated up front so the sort key and
    // duplicate check below can use `expect` instead of re-reporting
    // parse errors.
    for (name, _, _) in entries.iter() {
        name.parse::<usize>()
            .map_err(|_| DeserializeError::InvalidOrdinal(name.clone()))?;
    }
    entries
        .sort_by_cached_key(|(name, _, _)| name.parse::<usize>().expect("ordinal validated above"));
    for pair in entries.windows(2) {
        let a = pair[0].0.parse::<usize>().expect("ordinal validated above");
        let b = pair[1].0.parse::<usize>().expect("ordinal validated above");
        if a == b {
            return Err(DeserializeError::DuplicateOrdinal(a));
        }
    }
    Ok(())
}

/// The `k`/`v` object ids of a composite-key map pair sub-tree, shared by
/// the typed and schema-driven `Map` branches.
pub(crate) fn map_pair_entries(
    pair: &[(String, ObjectId, EntryKind)],
) -> Result<(ObjectId, ObjectId), DeserializeError> {
    let find = |want: &'static str| {
        pair.iter()
            .find(|(n, _, _)| n == want)
            .map(|(_, o, _)| *o)
            .ok_or(DeserializeError::MissingMapPairEntry { entry: want })
    };
    Ok((find("k")?, find("v")?))
}

/// Validate an `Option` tree's entries, returning the `some` entry's object
/// id, or `None` for the marker tree written for `None`. Legacy mode also
/// accepts the historical literal empty tree as `None`.
pub(crate) fn validate_option_entries(
    entries: &[(String, ObjectId, EntryKind)],
    mode: DecodeMode,
) -> Result<Option<ObjectId>, DeserializeError> {
    if crate::marker::is_marker(entries)
        || (matches!(mode, DecodeMode::LegacyLeaves) && entries.is_empty())
    {
        return Ok(None);
    }
    let [(name, inner_oid, _)] = entries else {
        return Err(DeserializeError::MalformedOption {
            found: entries.len(),
        });
    };
    if name != "some" {
        return Err(DeserializeError::MislabeledOption { name: name.clone() });
    }
    Ok(Some(*inner_oid))
}

/// Extract an enum value's (variant-name, payload object id) pair from the
/// object at `oid`, shared by the typed and schema-driven enum branches.
///
/// A unit variant's tag is a bare blob holding the variant name (returning
/// `None` for the payload); every other variant's tag is a single-entry tree
/// (returning `Some` of the payload id). The caller checks the returned form
/// against the named variant's own kind; this function has no variant table.
pub(crate) fn extract_enum_entry_mode<F: Find + ?Sized>(
    oid: &ObjectId,
    store: &F,
    mode: DecodeMode,
) -> Result<(String, Option<ObjectId>), DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object(oid, &mut buf, store)?;
    if data.kind == Kind::Blob {
        let bytes = strip_leaf_newline(oid, data.data.to_owned(), mode)?;
        let name = String::from_utf8(bytes).map_err(|_| DeserializeError::NonUtf8Blob(*oid))?;
        return Ok((name, None));
    }
    let entries = tree_entries_from_data(&data, oid)?;
    if entries.len() != 1 {
        return Err(DeserializeError::MalformedEnum {
            found: entries.len(),
        });
    }
    let (name, inner_oid, _) = entries
        .into_iter()
        .next()
        .expect("length checked to be 1 above");
    Ok((name, Some(inner_oid)))
}

/// Build a scalar-keyed map's key value from its entry name's textual form.
///
/// The key's shape may still be wrapped (an `Arc<str>` key collapses to
/// `str` for the scalar-vs-composite decision but the `Partial` frame stays
/// shaped `Arc<str>`, with no direct parse function), so this unwraps
/// smart-pointer and transparent-newtype layers before `parse_from_str` —
/// without a separate key object to fetch, since the entry name already is
/// the key's textual form.
fn parse_key_from_str<'facet>(
    partial: Partial<'facet, true>,
    text: &str,
) -> Result<Partial<'facet, true>, DeserializeError> {
    let shape = partial.shape();
    if let Def::Pointer(_) = shape.def {
        let partial = partial.begin_smart_ptr().map_err(reflect)?;
        let partial = parse_key_from_str(partial, text)?;
        return partial.end().map_err(reflect);
    }
    if shape.inner.is_some() && shape.vtable.has_try_borrow_inner() {
        let partial = partial.begin_inner().map_err(reflect)?;
        let partial = parse_key_from_str(partial, text)?;
        return partial.end().map_err(reflect);
    }
    partial
        .parse_from_str(text)
        .map_err(|e| DeserializeError::Parse {
            shape: shape.type_identifier,
            text: text.to_owned(),
            reason: e.to_string(),
        })
}

/// Whether an absent field of this shape reads as `None` rather than
/// [`DeserializeError::MissingField`]; checked on the field's own shape,
/// before any transparent collapse.
///
/// [`classify`]: crate::classify::classify
fn is_optional_shape(shape: &facet::Shape) -> bool {
    matches!(shape.def, Def::Option(_))
}

/// Begin `field`, set its default, and end it: the typed read of an absent
/// `Option` field.
fn set_field_default<'facet>(
    partial: Partial<'facet, true>,
    field: &facet::Field,
) -> Result<Partial<'facet, true>, DeserializeError> {
    let partial = partial
        .begin_field(field.name)
        .map_err(|e| DeserializeError::Reflect(format!("begin_field {}: {e}", field.name)))?;
    let partial = partial.set_default().map_err(reflect)?;
    partial
        .end()
        .map_err(|e| DeserializeError::Reflect(format!("end field {}: {e}", field.name)))
}

fn deser_into<'facet, F: Find + ?Sized>(
    partial: Partial<'facet, true>,
    oid: &ObjectId,
    store: &F,
    depth: usize,
    mode: DecodeMode,
) -> Result<Partial<'facet, true>, DeserializeError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH));
    }
    let shape = partial.shape();

    // RawTree: capture the child entry's object id without decoding its
    // contents; verify kind so a malformed or foreign tree fails fast.
    if matches!(classify(shape), ShapeClass::RawTree) {
        let mut buf = Vec::new();
        let data = find_object(oid, &mut buf, store)?;
        if data.kind != Kind::Tree {
            return Err(DeserializeError::NotATree(*oid));
        }
        return partial.set(RawTree::new(*oid)).map_err(reflect);
    }

    // RawBlob: capture the child entry's object id; verify kind so a
    // malformed or foreign tree fails fast.
    if matches!(classify(shape), ShapeClass::RawBlob) {
        let mut buf = Vec::new();
        let data = find_object(oid, &mut buf, store)?;
        if data.kind != Kind::Blob {
            return Err(DeserializeError::NotABlob(*oid));
        }
        return partial.set(RawBlob::new(*oid)).map_err(reflect);
    }

    // Dynamic value (`Def::DynamicValue`, e.g. `facet_value::Value`): the
    // encoding writes no type marker, so the value's shape is recovered
    // heuristically from the object graph itself.
    if matches!(classify(shape), ShapeClass::Dynamic) {
        return deser_dynamic(partial, oid, store, depth, mode);
    }

    // Scalar leaf: read blob, parse from str
    if matches!(classify(shape), ShapeClass::Scalar) {
        let bytes = find_blob_bytes_mode(oid, store, mode)?;
        let s = std::str::from_utf8(&bytes).map_err(|_| DeserializeError::NonUtf8Blob(*oid))?;
        return partial
            .parse_from_str(s)
            .map_err(|e| DeserializeError::Parse {
                shape: shape.type_identifier,
                text: s.to_owned(),
                reason: e.to_string(),
            });
    }

    // Byte sequence (`Vec<u8>`, `[u8; N]`): read the single blob. An exact
    // `Vec<u8>` target takes the whole buffer in one set; other byte-leaf
    // shapes fill item by item below.
    if matches!(classify(shape), ShapeClass::Bytes) {
        let bytes = find_blob_bytes_mode(oid, store, mode)?;
        if shape.is_type::<Vec<u8>>() {
            return partial.set::<Vec<u8>>(bytes).map_err(reflect);
        }
        if matches!(shape.def, Def::Array(_)) {
            let mut partial = partial.init_array().map_err(reflect)?;
            for (i, b) in bytes.iter().enumerate() {
                partial = partial.begin_nth_field(i).map_err(reflect)?;
                partial = partial.set::<u8>(*b).map_err(reflect)?;
                partial = partial.end().map_err(reflect)?;
            }
            return Ok(partial);
        }
        let mut partial = partial.init_list().map_err(reflect)?;
        for b in bytes {
            partial = partial.begin_list_item().map_err(reflect)?;
            partial = partial.set::<u8>(b).map_err(reflect)?;
            partial = partial.end().map_err(reflect)?;
        }
        return Ok(partial);
    }

    // Smart pointer (`Box`/`Arc`/`Rc`). For a slice pointee (`Arc<[T]>`) facet
    // hands back a slice builder we feed item by item; its element type decides
    // blob-vs-tree exactly as for an owned sequence. For a sized pointee the
    // pointee shares this node's encoding, so we recurse on the same object.
    if let Def::Pointer(pd) = shape.def {
        let mut partial = partial.begin_smart_ptr().map_err(reflect)?;
        if partial.is_building_smart_ptr_slice() {
            if pd.pointee.is_some_and(is_byte_seq) {
                let bytes = find_blob_bytes_mode(oid, store, mode)?;
                for b in bytes {
                    partial = partial.begin_list_item().map_err(reflect)?;
                    partial = partial.set::<u8>(b).map_err(reflect)?;
                    partial = partial.end().map_err(reflect)?;
                }
            } else {
                let mut entries = find_tree_entries(oid, store)?;
                if crate::marker::is_marker(&entries) {
                    entries.clear();
                }
                sort_by_ordinal(&mut entries)?;
                for (_, child_oid, _) in entries {
                    partial = partial.begin_list_item().map_err(reflect)?;
                    partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
                    partial = partial.end().map_err(reflect)?;
                }
            }
            return partial.end().map_err(reflect);
        }
        partial = deser_into(partial, oid, store, depth + 1, mode)?;
        return partial.end().map_err(reflect);
    }

    // Transparent newtype (`#[facet(transparent)]`, `NonZero<T>`, path
    // wrappers): the object was written as the inner value's own encoding, so
    // build that and let `begin_inner` reassemble the wrapper. Gated on
    // `has_try_borrow_inner` because plain collections like `Vec<T>` also
    // carry an `inner` shape but were never unwrapped on serialization.
    if shape.inner.is_some() && shape.vtable.has_try_borrow_inner() {
        let partial = partial.begin_inner().map_err(reflect)?;
        let partial = deser_into(partial, oid, store, depth + 1, mode)?;
        return partial.end().map_err(reflect);
    }

    // Struct: read tree, fill fields by name. Tuples and tuple structs key their
    // entries by zero-padded positional ordinal (mirroring serialization).
    if matches!(classify(shape), ShapeClass::Struct)
        && let facet::Type::User(facet::UserType::Struct(st)) = shape.ty
    {
        let positional = matches!(
            st.kind,
            facet::StructKind::Tuple | facet::StructKind::TupleStruct
        );
        let entries = find_tree_entries(oid, store)?;
        let mut matched = vec![false; entries.len()];
        let mut partial = partial;
        for (i, field) in st.fields.iter().enumerate() {
            let entry_name = if positional {
                format!("{i:04}")
            } else {
                field.name.to_string()
            };
            match entries.iter().position(|(name, _, _)| *name == entry_name) {
                Some(pos) => {
                    let child_oid = entries[pos].1;
                    matched[pos] = true;
                    partial = partial.begin_field(field.name).map_err(|e| {
                        DeserializeError::Reflect(format!("begin_field {}: {e}", field.name))
                    })?;
                    partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
                    partial = partial.end().map_err(|e| {
                        DeserializeError::Reflect(format!("end field {}: {e}", field.name))
                    })?;
                }
                // Absent `Option` fields read as `None`; any other missing
                // field means the tree does not describe this type.
                None if field.has_default() => {}
                None if is_optional_shape(field.shape()) => {
                    partial = set_field_default(partial, field)?;
                }
                None => return Err(DeserializeError::MissingField { field: entry_name }),
            }
        }
        for (pos, m) in matched.iter().enumerate() {
            if !*m && !crate::schema::pin::is_splice_entry(&entries[pos].0) {
                return Err(DeserializeError::UnexpectedEntry {
                    entry: entries[pos].0.clone(),
                });
            }
        }
        return Ok(partial);
    }

    // List (Vec): read tree with ordinal keys, sort numerically, push items.
    // The marker tree written for an empty list is stripped first.
    if matches!(classify(shape), ShapeClass::Sequence) && matches!(shape.def, Def::List(_)) {
        let mut entries = find_tree_entries(oid, store)?;
        if crate::marker::is_marker(&entries) {
            entries.clear();
        }
        sort_by_ordinal(&mut entries)?;
        let mut partial = partial.init_list().map_err(reflect)?;
        for (_, child_oid, _) in entries {
            partial = partial.begin_list_item().map_err(reflect)?;
            partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
            partial = partial.end().map_err(reflect)?;
        }
        return Ok(partial);
    }

    // Array: same as List but init_array
    if matches!(classify(shape), ShapeClass::Sequence) && matches!(shape.def, Def::Array(_)) {
        let mut entries = find_tree_entries(oid, store)?;
        if crate::marker::is_marker(&entries) {
            entries.clear();
        }
        sort_by_ordinal(&mut entries)?;
        let mut partial = partial.init_array().map_err(reflect)?;
        for (name, child_oid, _) in entries {
            let idx = name
                .parse::<usize>()
                .expect("ordinal validated by sort_by_ordinal");
            partial = partial.begin_nth_field(idx).map_err(reflect)?;
            partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
            partial = partial.end().map_err(reflect)?;
        }
        return Ok(partial);
    }

    // Map: mirror serialization. Scalar-keyed maps name each entry by the
    // key's textual form (parsed back via `parse_from_str`); composite-keyed
    // maps store each pair as a `{ k, v }` sub-tree. The scalar-vs-composite
    // decision uses the key shape after transparency collapse, matching what
    // the encoder wrote.
    if matches!(classify(shape), ShapeClass::Map)
        && let Def::Map(md) = shape.def
    {
        let mut entries = find_tree_entries(oid, store)?;
        if crate::marker::is_marker(&entries) {
            entries.clear();
        }
        let scalar_keys = matches!(collapse_shape(md.k).def, Def::Scalar);
        let mut partial = partial.init_map().map_err(reflect)?;
        if scalar_keys {
            for (key, child_oid, _) in entries {
                partial = partial.begin_key().map_err(reflect)?;
                partial = parse_key_from_str(partial, &key)?;
                partial = partial.end().map_err(reflect)?;
                partial = partial.begin_value().map_err(reflect)?;
                partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
                partial = partial.end().map_err(reflect)?;
            }
        } else {
            let mut key_oids = BTreeSet::new();
            for (_, pair_oid, _) in entries {
                let pair = find_tree_entries(&pair_oid, store)?;
                let (k_oid, v_oid) = map_pair_entries(&pair)?;
                if !key_oids.insert(k_oid) {
                    return Err(DeserializeError::DuplicateKey { oid: k_oid });
                }
                partial = partial.begin_key().map_err(reflect)?;
                partial = deser_into(partial, &k_oid, store, depth + 1, mode)?;
                partial = partial.end().map_err(reflect)?;
                partial = partial.begin_value().map_err(reflect)?;
                partial = deser_into(partial, &v_oid, store, depth + 1, mode)?;
                partial = partial.end().map_err(reflect)?;
            }
        }
        return Ok(partial);
    }

    // Option: empty tree → None, single "some"-named entry → Some(inner).
    if matches!(classify(shape), ShapeClass::Option) {
        let entries = find_tree_entries(oid, store)?;
        let Some(inner_oid) = validate_option_entries(&entries, mode)? else {
            // None — the partial already holds the default None.
            return Ok(partial);
        };
        let partial = partial.begin_some().map_err(reflect)?;
        let partial = deser_into(partial, &inner_oid, store, depth + 1, mode)?;
        return partial.end().map_err(reflect);
    }

    // Enum: externally tagged. A unit variant's tag is a bare blob holding
    // the variant name; every other variant's tag is a single-entry tree
    // (variant name → payload).
    if matches!(classify(shape), ShapeClass::Enum)
        && let facet::Type::User(facet::UserType::Enum(et)) = shape.ty
    {
        let (variant_name, inner_oid) = extract_enum_entry_mode(oid, store, mode)?;

        // The variant's field layout comes from the type, not the tree: a tuple
        // variant (`TupleStruct`) keys by ordinal, a struct variant by name, and a
        // newtype (single-field tuple) variant resolves directly to its field.
        let variant = et.variants.iter().find(|v| v.name == variant_name);
        let positional =
            variant.is_some_and(|v| matches!(v.data.kind, facet::StructKind::TupleStruct));
        let newtype = positional && variant.is_some_and(|v| v.data.fields.len() == 1);
        let is_unit = variant.is_some_and(|v| v.data.fields.is_empty());

        // `select_variant_named` has already rejected an unknown name.
        let mut partial = partial.select_variant_named(&variant_name).map_err(|e| {
            DeserializeError::Reflect(format!("select variant {variant_name}: {e}"))
        })?;
        let Some(variant) = variant else {
            // Unreachable unless this lookup and `select_variant_named` disagree.
            return Err(DeserializeError::Reflect(format!(
                "unknown variant {variant_name}"
            )));
        };

        let inner_oid = match (is_unit, inner_oid) {
            // Unit variant: the variant name already consumed by
            // `select_variant_named` is the payload's entire content.
            (true, None) => return Ok(partial),
            (true, Some(_)) => {
                return Err(DeserializeError::UnitVariantIsTree {
                    variant: variant_name,
                });
            }
            (false, Some(inner_oid)) => inner_oid,
            (false, None) => {
                return Err(DeserializeError::VariantPayloadIsBlob {
                    variant: variant_name,
                });
            }
        };

        if newtype {
            partial = partial.begin_nth_field(0).map_err(reflect)?;
            partial = deser_into(partial, &inner_oid, store, depth + 1, mode)?;
            return partial.end().map_err(reflect);
        }

        let inner_entries = find_tree_entries(&inner_oid, store)?;
        let fields = &variant.data.fields;
        let mut matched = vec![false; inner_entries.len()];
        for (i, field) in fields.iter().enumerate() {
            let entry_name = if positional {
                format!("{i:04}")
            } else {
                field.name.to_string()
            };
            match inner_entries
                .iter()
                .position(|(name, _, _)| *name == entry_name)
            {
                Some(pos) => matched[pos] = true,
                None if field.has_default() => {}
                None if is_optional_shape(field.shape()) => {
                    partial = set_field_default(partial, field)?;
                }
                None => return Err(DeserializeError::MissingField { field: entry_name }),
            }
        }
        for (pos, m) in matched.iter().enumerate() {
            if !*m && !crate::schema::pin::is_splice_entry(&inner_entries[pos].0) {
                return Err(DeserializeError::UnexpectedEntry {
                    entry: inner_entries[pos].0.clone(),
                });
            }
        }
        for (name, child_oid, _) in inner_entries {
            if positional {
                let idx = name
                    .parse::<usize>()
                    .map_err(|_| DeserializeError::InvalidOrdinal(name.clone()))?;
                partial = partial.begin_nth_field(idx).map_err(reflect)?;
            } else {
                partial = partial.begin_field(&name).map_err(reflect)?;
            }
            partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
            partial = partial.end().map_err(reflect)?;
        }
        return Ok(partial);
    }

    Err(DeserializeError::Unsupported(shape.type_identifier))
}

/// Decode the object at `oid` into a dynamic value (`Def::DynamicValue`).
///
/// The encoding writes no type markers, so the value's shape is recovered
/// by a documented lossy heuristic:
///
/// - a blob is a `String` when valid UTF-8 (after stripping the mandatory
///   trailing newline), otherwise `Bytes`;
/// - a non-empty tree whose entry names are all decimal ordinals is an Array;
/// - any other tree — including the presence-marker tree written for `Null`
///   and an empty `Array`/`Object` — is an Object; legacy mode additionally
///   reads a literal empty tree as null.
///
/// So bool, numbers, char, datetime, … come back as `String`s of their
/// textual form, and null as an empty `Object`. The depth guard is enforced
/// by the caller ([`deser_into`]); children recurse through it at
/// `depth + 1`.
fn deser_dynamic<'facet, F: Find + ?Sized>(
    partial: Partial<'facet, true>,
    oid: &ObjectId,
    store: &F,
    depth: usize,
    mode: DecodeMode,
) -> Result<Partial<'facet, true>, DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object(oid, &mut buf, store)?;

    // Blob → String or Bytes, decided by UTF-8 validity (of the content with
    // its mandatory trailing newline already stripped).
    if data.kind == Kind::Blob {
        let bytes = strip_leaf_newline(oid, data.data.to_owned(), mode)?;
        return match String::from_utf8(bytes) {
            Ok(s) => partial.set::<String>(s).map_err(reflect),
            Err(e) => partial.set::<Vec<u8>>(e.into_bytes()).map_err(reflect),
        };
    }

    let mut entries = tree_entries_from_data(&data, oid)?;
    let marker = crate::marker::is_marker(&entries);
    if marker {
        entries.clear();
    }
    if matches!(mode, DecodeMode::LegacyLeaves) && !marker && entries.is_empty() {
        return partial.set::<Value>(Value::NULL).map_err(reflect);
    }

    // Non-empty + all-ordinal names → Array. In strict mode, and for the
    // current marker tree in legacy mode, an empty tree reads as an Object:
    // it is what both null and the empty Object serialize to, and an empty
    // Object is the less lossy of the two readings.
    let all_ordinal = !entries.is_empty()
        && entries
            .iter()
            .all(|(name, _, _)| name.parse::<usize>().is_ok());
    if all_ordinal {
        sort_by_ordinal(&mut entries)?;
        let mut partial = partial.init_list().map_err(reflect)?;
        for (_, child_oid, _) in entries {
            partial = partial.begin_list_item().map_err(reflect)?;
            partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
            partial = partial.end().map_err(reflect)?;
        }
        return Ok(partial);
    }

    let mut partial = partial.init_map().map_err(reflect)?;
    for (name, child_oid, _) in entries {
        partial = partial.begin_object_entry(&name).map_err(reflect)?;
        partial = deser_into(partial, &child_oid, store, depth + 1, mode)?;
        partial = partial.end().map_err(reflect)?;
    }
    Ok(partial)
}
