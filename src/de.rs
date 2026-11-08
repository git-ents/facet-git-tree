//! Deserialization of bare Git objects into [`facet::Facet`] values.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use facet::{Def, Partial, Peek, ScalarType};
use gix_object::{Data, Find, Kind};

pub(crate) use crate::classify::collapse_shape;
use crate::classify::{ShapeClass, classify, is_byte_seq};
use crate::error::{DeserializeError, KeyError};
use crate::limits::MAX_VALUE_DEPTH;
use crate::name::{self, NameError};
use crate::ordinal::{self, MAX_TREE_ENTRIES, OrdinalError};
use crate::{EntryKind, ObjectId, RawBlob, RawTree};

/// Each object occurrence, including a marker blob, consumes one visit.
pub(crate) const MAX_DECODE_VISITS: usize = 1 << 20;

/// A single budget belongs to the whole decode, not to a particular object id.
#[derive(Debug, Default)]
pub(crate) struct DecodeBudget {
    visits: usize,
    cbor_blobs: BTreeMap<ObjectId, usize>,
    cbor_shapes: BTreeMap<facet::ConstTypeId, bool>,
    cbor_hash_kinds: BTreeSet<gix_hash::Kind>,
}

impl DecodeBudget {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn visit(&mut self, depth: usize) -> Result<(), DeserializeError> {
        check_depth(depth)?;
        if self.visits == MAX_DECODE_VISITS {
            return Err(reflect(format!(
                "maximum decode visits ({MAX_DECODE_VISITS}) exceeded"
            )));
        }
        self.visits += 1;
        Ok(())
    }
}

pub(crate) fn check_depth(depth: usize) -> Result<(), DeserializeError> {
    if depth > MAX_VALUE_DEPTH {
        Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH))
    } else {
        Ok(())
    }
}

fn reflect(error: impl Display) -> DeserializeError {
    DeserializeError::Reflect(error.to_string())
}

/// Check whether a user name has a representable encoded tree-entry name.
///
/// Names requiring escapes, including the empty string, are valid. The limit
/// applies to the encoded name rather than the original UTF-8 string.
pub fn check_key(key: &str) -> Result<(), KeyError> {
    crate::name::escape(key)
        .map(|_| ())
        .map_err(|_error| KeyError {
            key: key.to_owned(),
        })
}

/// Deserialize a value from a bare object id. Scalar roots can be blobs.
///
/// This does not read or expect the git-store `.schema` envelope. Dynamic
/// values are decoded using an intentionally lossy, untagged heuristic.
pub fn deserialize<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
) -> Result<T, DeserializeError> {
    let mut budget = DecodeBudget::new();
    deserialize_with_budget(root, store, 0, &mut budget, None)
}

/// Continue a schema-directed walk without resetting its object-visit budget.
/// `expected` is the enclosing tree entry's mode, or `None` at a bare root.
pub(crate) fn deserialize_with_budget<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<T, DeserializeError> {
    let partial =
        Partial::alloc::<T>().map_err(|error| reflect(format!("alloc failed: {error}")))?;
    let partial = deser_into(partial, root, store, depth, budget, expected, 0)?;
    let heap = partial
        .build()
        .map_err(|error| reflect(format!("build failed: {error}")))?;
    if !budget.cbor_blobs.is_empty() {
        validate_cbor_fixed_points(heap.peek(), budget, depth)?;
        if !budget.cbor_blobs.is_empty() {
            return Err(reflect(
                "decoded CBOR blobs were lost during reconstruction",
            ));
        }
    }
    heap.materialize::<T>()
        .map_err(|error| reflect(format!("materialize failed: {error}")))
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

/// Fetch and count one encoded object; verify its parent's declared mode.
pub(crate) fn find_object_with_budget<'a, F: Find + ?Sized>(
    id: &ObjectId,
    buf: &'a mut Vec<u8>,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<Data<'a>, DeserializeError> {
    budget.visit(depth)?;
    let data = find_object(id, buf, store)?;
    if data.object_hash != id.kind() {
        return Err(reflect(format!("object {id} has the wrong hash format")));
    }
    match expected {
        Some(LeafKind::Blob) if data.kind != Kind::Blob => {
            return Err(DeserializeError::NotABlob(*id));
        }
        Some(LeafKind::Tree) if data.kind != Kind::Tree => {
            return Err(DeserializeError::NotATree(*id));
        }
        Some(LeafKind::Blob | LeafKind::Tree) | None => {}
    }
    Ok(data)
}

/// A single validated tree entry: its name, target object, and object kind.
///
/// The parser builds one per tree entry and is the only place an entry name is
/// unescaped or a mode is classified, so downstream code handles only these
/// already-validated forms.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) name: EncodedName,
    pub(crate) oid: ObjectId,
    pub(crate) kind: LeafKind,
}

/// A validated tree-entry name.
///
/// A name is either a structural marker name (`none`/`some`) or a canonical
/// encoded user name paired with its once-unescaped form. `parse` performs the
/// only unescaping; consumers read [`EncodedName::user`] and can neither
/// re-decode the encoded bytes nor re-derive the user name.
#[derive(Debug, Clone)]
pub(crate) struct EncodedName(NameKind);

/// The private shape behind [`EncodedName`].
#[derive(Debug, Clone)]
enum NameKind {
    /// A structural marker name, which is never a user name.
    Structural(&'static str),
    /// A canonical encoded user name and its once-unescaped form.
    User { encoded: String, decoded: String },
}

impl EncodedName {
    /// Validate an untrusted wire name, unescaping a user name exactly once.
    ///
    /// `none` and `some` are structural marker names rather than user names and
    /// are stored verbatim; every other reserved name is refused.
    fn parse(encoded: &str) -> Result<Self, NameError> {
        match name::unescape(encoded) {
            Ok(decoded) => Ok(Self(NameKind::User {
                encoded: encoded.to_owned(),
                decoded,
            })),
            Err(NameError::Structural) if encoded == crate::marker::MARKER_KEY => {
                Ok(Self(NameKind::Structural(crate::marker::MARKER_KEY)))
            }
            Err(NameError::Structural) if encoded == crate::marker::SOME_KEY => {
                Ok(Self(NameKind::Structural(crate::marker::SOME_KEY)))
            }
            Err(error) => Err(error),
        }
    }

    /// The encoded wire name, exactly as stored in the tree.
    pub(crate) fn encoded(&self) -> &str {
        match self {
            Self(NameKind::Structural(name)) => name,
            Self(NameKind::User { encoded, .. }) => encoded,
        }
    }

    /// The once-unescaped user name, or `None` for a structural marker name.
    pub(crate) fn user(&self) -> Option<&str> {
        match self {
            Self(NameKind::Structural(_)) => None,
            Self(NameKind::User { decoded, .. }) => Some(decoded),
        }
    }
}

/// The only two object kinds a validated tree entry may name.
///
/// Every other Git mode — executable blobs, links, and commits — is rejected
/// by the parser, so downstream code never re-matches those unreachable cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeafKind {
    /// A regular file, mode `100644`.
    Blob,
    /// A sub-tree, mode `40000`.
    Tree,
}

/// Parse and validate one already-fetched tree, without consuming another visit.
///
/// Tree mode 040000 uses Git's canonical `40000` wire spelling; a regular
/// blob uses `100644`. Other permissions, kinds and mode spellings are rejected.
pub(crate) fn tree_entries_from_data(
    data: &Data<'_>,
    id: &ObjectId,
) -> Result<Vec<Entry>, DeserializeError> {
    if data.kind != Kind::Tree {
        return Err(DeserializeError::NotATree(*id));
    }
    if data.object_hash != id.kind() {
        return Err(reflect(format!("object {id} has the wrong hash format")));
    }
    // gix's octal parser can overflow on malicious mode bytes. Validate the
    // raw spelling before invoking it, and bound the scan and allocation.
    let mut remaining = data.data;
    let mut entry_count = 0usize;
    while !remaining.is_empty() {
        if entry_count == MAX_TREE_ENTRIES {
            return Err(reflect(format!(
                "tree {id} exceeds the maximum of {MAX_TREE_ENTRIES} entries"
            )));
        }
        entry_count += 1;
        let mode_len = remaining
            .iter()
            .take(7)
            .position(|&byte| byte == b' ')
            .ok_or_else(|| reflect(format!("tree {id} has a missing or overlong entry mode")))?;
        let mode_bytes = remaining.get(..mode_len).ok_or(DeserializeError::Decode {
            oid: *id,
            source: gix_object::decode::Error,
        })?;
        if !matches!(mode_bytes, b"100644" | b"40000") {
            return Err(reflect(format!(
                "tree {id} has invalid or non-canonical entry mode {mode_bytes:?}"
            )));
        }
        let after_mode = remaining
            .get(mode_len + 1..)
            .ok_or(DeserializeError::Decode {
                oid: *id,
                source: gix_object::decode::Error,
            })?;
        let name_len = after_mode
            .iter()
            .take(crate::name::MAX_ENCODED_NAME_BYTES + 1)
            .position(|&byte| byte == 0)
            .ok_or_else(|| reflect(format!("tree {id} has a missing or overlong name")))?;
        remaining = after_mode
            .get(name_len + 1 + data.object_hash.len_in_bytes()..)
            .ok_or(DeserializeError::Decode {
                oid: *id,
                source: gix_object::decode::Error,
            })?;
    }

    let mut result = Vec::with_capacity(entry_count);
    let mut names = BTreeSet::new();
    let mut previous: Option<gix_object::tree::EntryRef<'_>> = None;
    for entry in gix_object::TreeRefIter::from_bytes(data.data, data.object_hash) {
        let entry = entry.map_err(|source| DeserializeError::Decode { oid: *id, source })?;
        // The raw mode scan above admitted only `100644` and `40000`, so this
        // classification cannot see any other kind; the arm is a defensive
        // boundary check rather than a downstream re-match.
        let kind = match entry.mode.kind() {
            EntryKind::Blob => LeafKind::Blob,
            EntryKind::Tree => LeafKind::Tree,
            other => {
                return Err(reflect(format!(
                    "tree {id} has invalid entry kind {other:?}"
                )));
            }
        };
        let raw = std::str::from_utf8(entry.filename).map_err(|_error| {
            DeserializeError::NonUtf8Name(String::from_utf8_lossy(entry.filename).into_owned())
        })?;
        let name = EncodedName::parse(raw)
            .map_err(|error| reflect(format!("invalid encoded name {raw:?}: {error}")))?;
        if !names.insert(raw) {
            return Err(reflect(format!(
                "tree {id} contains duplicate name {raw:?}"
            )));
        }
        if previous.is_some_and(|last| last.cmp(&entry) != std::cmp::Ordering::Less) {
            return Err(reflect(format!("tree {id} is not in Git entry order")));
        }
        previous = Some(entry);
        result.push(Entry {
            name,
            oid: entry.oid.to_owned(),
            kind,
        });
    }
    if result.is_empty() {
        return Err(reflect(format!("tree {id} is empty; use the none marker")));
    }
    Ok(result)
}

/// Fetch a strict tree and spend one visit for that object.
pub(crate) fn find_tree_entries<F: Find + ?Sized>(
    id: &ObjectId,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<Vec<Entry>, DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object_with_budget(id, &mut buf, store, depth, budget, expected)?;
    tree_entries_from_data(&data, id)
}

/// Fetch a binary blob verbatim, counting one object visit.
pub(crate) fn find_blob_raw<F: Find + ?Sized>(
    id: &ObjectId,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<Vec<u8>, DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object_with_budget(id, &mut buf, store, depth, budget, expected)?;
    if data.kind != Kind::Blob {
        return Err(DeserializeError::NotABlob(*id));
    }
    Ok(data.data.to_owned())
}

/// Fetch a text blob and remove exactly its final newline.
pub(crate) fn find_blob_bytes<F: Find + ?Sized>(
    id: &ObjectId,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<Vec<u8>, DeserializeError> {
    strip_leaf_newline(id, find_blob_raw(id, store, depth, budget, expected)?)
}

pub(crate) fn strip_leaf_newline(
    id: &ObjectId,
    mut bytes: Vec<u8>,
) -> Result<Vec<u8>, DeserializeError> {
    if bytes.pop() == Some(b'\n') {
        Ok(bytes)
    } else {
        Err(DeserializeError::MissingLeafNewline(*id))
    }
}

/// The entry's once-unescaped user name.
///
/// A structural marker name (`none` or `some`) is not a user name; reading one
/// where a user name is required is refused, exactly as the parser refuses
/// every other reserved name. The name is never re-decoded here.
fn user_name(name: &EncodedName) -> Result<&str, DeserializeError> {
    name.user().ok_or_else(|| {
        reflect(format!(
            "invalid encoded name {:?}: a structural name is not a user name",
            name.encoded()
        ))
    })
}

/// Validate and unescape a user name that is not a tree-entry name, such as the
/// contents of an enum tag blob.
fn user_name_from_encoded(encoded: &str) -> Result<String, DeserializeError> {
    let name = EncodedName::parse(encoded)
        .map_err(|error| reflect(format!("invalid encoded name {encoded:?}: {error}")))?;
    Ok(user_name(&name)?.to_owned())
}

/// Validate, rather than sort, the already Git-ordered ordinal entries.
pub(crate) fn sort_by_ordinal(entries: &[Entry]) -> Result<(), DeserializeError> {
    ordinal::validate_sequence(entries.iter().map(|entry| entry.name.encoded().as_bytes()))
        .map(|_| ())
        .map_err(|error| match error {
            OrdinalError::Duplicate { index } => DeserializeError::DuplicateOrdinal(index),
            OrdinalError::Gap { expected, found } => reflect(format!(
                "ordinal gap: expected {expected:03x}, found {found:03x}"
            )),
            _ => {
                let name = entries.iter().find_map(|entry| {
                    ordinal::decode(entry.name.encoded())
                        .err()
                        .map(|_| entry.name.encoded().to_owned())
                });
                match name {
                    Some(name) => DeserializeError::InvalidOrdinal(name),
                    None => reflect(error),
                }
            }
        })
}

/// Detect and verify a marker, including its actual empty blob target.
pub(crate) fn validate_marker<F: Find + ?Sized>(
    entries: &[Entry],
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
) -> Result<bool, DeserializeError> {
    let [entry] = entries else {
        return Ok(false);
    };
    if entry.name.encoded() != crate::marker::MARKER_KEY {
        return Ok(false);
    }
    if entry.kind != LeafKind::Blob {
        return Err(reflect("none marker must have blob mode"));
    }
    let bytes = find_blob_raw(&entry.oid, store, depth + 1, budget, Some(entry.kind))?;
    if !bytes.is_empty() {
        return Err(reflect(format!(
            "none marker target {} is not an empty blob",
            entry.oid
        )));
    }
    Ok(true)
}

/// Require the exact marker and visit its empty blob target.
pub(crate) fn expect_marker<F: Find + ?Sized>(
    entries: &[Entry],
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
) -> Result<(), DeserializeError> {
    if validate_marker(entries, store, depth, budget)? {
        Ok(())
    } else {
        Err(reflect("expected a single none marker entry"))
    }
}

/// Return None for a validated marker, or the present entry's id and mode.
/// The caller must pass that mode when visiting the inner object.
pub(crate) fn validate_option_entries<F: Find + ?Sized>(
    entries: &[Entry],
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
) -> Result<Option<(ObjectId, LeafKind)>, DeserializeError> {
    if validate_marker(entries, store, depth, budget)? {
        return Ok(None);
    }
    let [entry] = entries else {
        return Err(DeserializeError::MalformedOption {
            found: entries.len(),
        });
    };
    if entry.name.encoded() != crate::marker::SOME_KEY {
        return Err(DeserializeError::MislabeledOption {
            name: entry.name.encoded().to_owned(),
        });
    }
    Ok(Some((entry.oid, entry.kind)))
}

/// Read an enum tag, validating its name, tree and incoming mode.
/// The returned payload mode must be passed on when visiting that payload.
pub(crate) fn extract_enum_entry<F: Find + ?Sized>(
    oid: &ObjectId,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<(String, Option<(ObjectId, LeafKind)>), DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object_with_budget(oid, &mut buf, store, depth, budget, expected)?;
    if data.kind == Kind::Blob {
        let bytes = strip_leaf_newline(oid, data.data.to_owned())?;
        let encoded =
            std::str::from_utf8(&bytes).map_err(|_error| DeserializeError::NonUtf8Blob(*oid))?;
        return Ok((user_name_from_encoded(encoded)?, None));
    }
    let entries = tree_entries_from_data(&data, oid)?;
    let [entry] = entries.as_slice() else {
        return Err(DeserializeError::MalformedEnum {
            found: entries.len(),
        });
    };
    Ok((
        user_name(&entry.name)?.to_owned(),
        Some((entry.oid, entry.kind)),
    ))
}

/// Syntax common to all signed and unsigned canonical decimal integers.
pub(crate) fn is_canonical_integer(text: &str, signed: bool) -> bool {
    let digits = if let Some(rest) = text.strip_prefix('-') {
        if !signed || rest == "0" {
            return false;
        }
        rest
    } else {
        text
    };
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

fn parse_scalar<'facet>(
    partial: Partial<'facet, true>,
    oid: &ObjectId,
    bytes: &[u8],
) -> Result<Partial<'facet, true>, DeserializeError> {
    let shape = partial.shape();
    let text = std::str::from_utf8(bytes).map_err(|_error| DeserializeError::NonUtf8Blob(*oid))?;
    let invalid = |reason: &str| DeserializeError::Parse {
        shape: shape.type_identifier,
        text: text.to_owned(),
        reason: reason.to_owned(),
    };
    let Some(scalar) = shape.scalar_type() else {
        return Err(DeserializeError::Unsupported(shape.type_identifier));
    };
    match scalar {
        ScalarType::F32 => {
            let value = crate::float::parse_f32(text).map_err(|()| invalid("non-canonical f32"))?;
            partial.set(value).map_err(reflect)
        }
        ScalarType::F64 => {
            let value = crate::float::parse_f64(text).map_err(|()| invalid("non-canonical f64"))?;
            partial.set(value).map_err(reflect)
        }
        ScalarType::Bool if text != "true" && text != "false" => Err(invalid("non-canonical bool")),
        ScalarType::Char if text.chars().count() != 1 => {
            Err(invalid("expected one Unicode scalar"))
        }
        ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::ISize
            if !is_canonical_integer(text, true) =>
        {
            Err(invalid("non-canonical integer"))
        }
        ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::U128
        | ScalarType::USize
            if !is_canonical_integer(text, false) =>
        {
            Err(invalid("non-canonical integer"))
        }
        ScalarType::ISize if text.parse::<i64>().is_err() => Err(invalid("outside i64 range")),
        ScalarType::USize if text.parse::<u64>().is_err() => Err(invalid("outside u64 range")),
        ScalarType::Bool
        | ScalarType::Char
        | ScalarType::Str
        | ScalarType::String
        | ScalarType::CowStr
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::ISize
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::U128
        | ScalarType::USize => {
            partial
                .parse_from_str(text)
                .map_err(|error| DeserializeError::Parse {
                    shape: shape.type_identifier,
                    text: text.to_owned(),
                    reason: error.to_string(),
                })
        }
        _ => Err(DeserializeError::Unsupported(shape.type_identifier)),
    }
}

fn checked_collapse_shape(
    mut shape: &'static facet::Shape,
) -> Result<&'static facet::Shape, DeserializeError> {
    for _ in 0..=MAX_VALUE_DEPTH {
        if let Def::Pointer(pointer) = shape.def {
            shape = pointer
                .pointee
                .ok_or(DeserializeError::Unsupported(shape.type_identifier))?;
        } else if let Some(inner) = shape.inner.filter(|_| shape.vtable.has_try_borrow_inner()) {
            shape = inner;
        } else {
            return Ok(shape);
        }
    }
    Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH))
}

fn map_uses_cbor(map: facet::MapDef) -> Result<bool, DeserializeError> {
    let key = checked_collapse_shape(map.k)?;
    Ok(crate::attr::is_identity_key(map.k.attributes)
        || !matches!(
            key.scalar_type(),
            Some(ScalarType::Str | ScalarType::String | ScalarType::CowStr)
        ))
}

fn shape_contains_cbor(
    shape: &'static facet::Shape,
    visited: &mut BTreeSet<facet::ConstTypeId>,
    depth: usize,
) -> Result<bool, DeserializeError> {
    if depth > MAX_VALUE_DEPTH {
        return Ok(true);
    }
    let shape = checked_collapse_shape(shape)?;
    if !visited.insert(shape.id) {
        return Ok(false);
    }
    match classify(shape) {
        ShapeClass::Map => {
            let Def::Map(map) = shape.def else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            if map_uses_cbor(map)? {
                return Ok(true);
            }
            shape_contains_cbor(map.v, visited, depth + 1)
        }
        ShapeClass::Struct => {
            let facet::Type::User(facet::UserType::Struct(st)) = shape.ty else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            for field in st.fields {
                if shape_contains_cbor(field.shape(), visited, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ShapeClass::Sequence => match crate::classify::seq_elem(shape) {
            Some(elem) => shape_contains_cbor(elem, visited, depth + 1),
            None => Err(DeserializeError::Unsupported(shape.type_identifier)),
        },
        ShapeClass::Option => {
            let Def::Option(option) = shape.def else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            shape_contains_cbor(option.t, visited, depth + 1)
        }
        ShapeClass::Enum => {
            let facet::Type::User(facet::UserType::Enum(enumeration)) = shape.ty else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            for variant in enumeration.variants {
                for field in variant.data.fields {
                    if shape_contains_cbor(field.shape(), visited, depth + 1)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Match re-encoded CBOR leaves against the multiset of decoded blob IDs.
/// A partial cannot be peeked safely until its entire root has been built.
fn validate_cbor_fixed_points(
    peek: Peek<'_, '_>,
    budget: &mut DecodeBudget,
    depth: usize,
) -> Result<(), DeserializeError> {
    check_depth(depth)?;
    let shape = peek.shape();
    let contains_cbor = match budget.cbor_shapes.get(&shape.id) {
        Some(contains) => *contains,
        None => {
            let contains = shape_contains_cbor(shape, &mut BTreeSet::new(), 0)?;
            budget.cbor_shapes.insert(shape.id, contains);
            contains
        }
    };
    if !contains_cbor {
        return Ok(());
    }
    let peek = peek.innermost_peek();
    let shape = peek.shape();
    match classify(shape) {
        ShapeClass::Map => {
            let Def::Map(map) = shape.def else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            if map_uses_cbor(map)? {
                let bytes = crate::cbor::encode_peek(peek).map_err(|error| {
                    reflect(format!(
                        "cannot re-encode {} as CBOR: {error}",
                        shape.type_identifier
                    ))
                })?;
                let mut matching_oid = None;
                for &kind in &budget.cbor_hash_kinds {
                    let oid =
                        gix_object::compute_hash(kind, Kind::Blob, &bytes).map_err(reflect)?;
                    if budget.cbor_blobs.contains_key(&oid) {
                        matching_oid = Some(oid);
                        break;
                    }
                }
                let Some(oid) = matching_oid else {
                    return Err(reflect(format!(
                        "decoded CBOR for {} does not reproduce an input blob",
                        shape.type_identifier
                    )));
                };
                if let Some(count) = budget.cbor_blobs.get_mut(&oid) {
                    *count -= 1;
                    if *count == 0 {
                        budget.cbor_blobs.remove(&oid);
                    }
                }
                return Ok(());
            }
            for (_, value) in peek.into_map().map_err(reflect)?.iter() {
                validate_cbor_fixed_points(value, budget, depth + 1)?;
            }
            Ok(())
        }
        ShapeClass::Struct => {
            let structure = peek.into_struct().map_err(reflect)?;
            for index in 0..structure.field_count() {
                validate_cbor_fixed_points(
                    structure.field(index).map_err(reflect)?,
                    budget,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        ShapeClass::Sequence => {
            for value in peek.into_list_like().map_err(reflect)?.iter() {
                validate_cbor_fixed_points(value, budget, depth + 1)?;
            }
            Ok(())
        }
        ShapeClass::Option => {
            if let Some(value) = peek.into_option().map_err(reflect)?.value() {
                validate_cbor_fixed_points(value, budget, depth + 1)?;
            }
            Ok(())
        }
        ShapeClass::Enum => {
            let enumeration = peek.into_enum().map_err(reflect)?;
            let variant = enumeration.active_variant().map_err(reflect)?;
            let positional = matches!(
                variant.data.kind,
                facet::StructKind::Tuple | facet::StructKind::TupleStruct
            );
            let child_depth = if positional && variant.data.fields.len() == 1 {
                depth + 1
            } else {
                depth + 2
            };
            for index in 0..variant.data.fields.len() {
                let value = enumeration
                    .field(index)
                    .map_err(reflect)?
                    .ok_or_else(|| reflect("missing enum field after decoding"))?;
                validate_cbor_fixed_points(value, budget, child_depth)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn parse_key_from_str<'facet>(
    partial: Partial<'facet, true>,
    text: &str,
    wrapper_depth: usize,
) -> Result<Partial<'facet, true>, DeserializeError> {
    if wrapper_depth > MAX_VALUE_DEPTH {
        return Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH));
    }
    let shape = partial.shape();
    if shape.scalar_type() == Some(ScalarType::CowStr) {
        return partial
            .set(std::borrow::Cow::<'facet, str>::Owned(text.to_owned()))
            .map_err(reflect);
    }
    if matches!(shape.def, Def::Pointer(_)) {
        let partial = partial.begin_smart_ptr().map_err(reflect)?;
        let partial = parse_key_from_str(partial, text, wrapper_depth + 1)?;
        return partial.end().map_err(reflect);
    }
    if shape.inner.is_some() && shape.vtable.has_try_borrow_inner() {
        let partial = partial.begin_inner().map_err(reflect)?;
        let partial = parse_key_from_str(partial, text, wrapper_depth + 1)?;
        return partial.end().map_err(reflect);
    }
    partial
        .parse_from_str(text)
        .map_err(|error| DeserializeError::Parse {
            shape: shape.type_identifier,
            text: text.to_owned(),
            reason: error.to_string(),
        })
}

/// Match every field exactly once before beginning any reflected field.
fn match_fields(
    entries: &[Entry],
    fields: &'static [facet::Field],
    positional: bool,
    owner: &'static str,
) -> Result<Vec<(ObjectId, LeafKind)>, DeserializeError> {
    if fields.len() > MAX_TREE_ENTRIES {
        return Err(DeserializeError::Unsupported(owner));
    }
    let mut expected = BTreeMap::new();
    for (index, field) in fields.iter().enumerate() {
        let name = if positional {
            ordinal::encode(index).map_err(reflect)?
        } else {
            crate::name::escape(field.effective_name()).map_err(reflect)?
        };
        if expected.insert(name.clone(), index).is_some() {
            return Err(reflect(format!("duplicate field name {name:?} in {owner}")));
        }
    }
    let mut matched = vec![None; fields.len()];
    for entry in entries {
        if !positional {
            user_name(&entry.name)?;
        }
        let Some(index) = expected.get(entry.name.encoded()) else {
            return Err(DeserializeError::UnexpectedEntry {
                entry: entry.name.encoded().to_owned(),
            });
        };
        matched[*index] = Some((entry.oid, entry.kind));
    }
    for (name, index) in expected {
        if matched[index].is_none() {
            return Err(DeserializeError::MissingField { field: name });
        }
    }
    matched
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| reflect("field matching did not initialize every field"))
}

fn deser_fields<'facet, F: Find + ?Sized>(
    mut partial: Partial<'facet, true>,
    mut entries: Vec<Entry>,
    fields: &'static [facet::Field],
    positional: bool,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
) -> Result<Partial<'facet, true>, DeserializeError> {
    if validate_marker(&entries, store, depth, budget)? {
        entries.clear();
    }
    if positional {
        sort_by_ordinal(&entries)?;
    }
    let owner = partial.shape().type_identifier;
    let matched = match_fields(&entries, fields, positional, owner)?;
    for (index, (oid, kind)) in matched.into_iter().enumerate() {
        partial = partial.begin_nth_field(index).map_err(reflect)?;
        partial = deser_into(partial, &oid, store, depth + 1, budget, Some(kind), 0)?;
        partial = partial.end().map_err(reflect)?;
    }
    Ok(partial)
}

fn check_length(expected: usize, found: usize) -> Result<(), DeserializeError> {
    if expected != found {
        return Err(reflect(format!(
            "sequence length mismatch: expected {expected}, found {found}"
        )));
    }
    Ok(())
}

fn deser_into<'facet, F: Find + ?Sized>(
    partial: Partial<'facet, true>,
    oid: &ObjectId,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
    wrapper_depth: usize,
) -> Result<Partial<'facet, true>, DeserializeError> {
    check_depth(depth)?;
    if wrapper_depth > MAX_VALUE_DEPTH {
        return Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH));
    }
    let shape = partial.shape();
    if shape.scalar_type() == Some(ScalarType::CowStr) {
        let bytes = find_blob_bytes(oid, store, depth, budget, expected)?;
        let text =
            String::from_utf8(bytes).map_err(|_error| DeserializeError::NonUtf8Blob(*oid))?;
        return partial
            .set(std::borrow::Cow::<'facet, str>::Owned(text))
            .map_err(reflect);
    }

    // Wrapper frames share the same encoded object and therefore consume no
    // additional object visit or encoded-tree depth.
    if let Def::Pointer(pointer) = shape.def {
        let mut partial = partial.begin_smart_ptr().map_err(reflect)?;
        if partial.is_building_smart_ptr_slice() {
            if pointer.pointee.is_some_and(is_byte_seq) {
                let bytes = find_blob_raw(oid, store, depth, budget, expected)?;
                for byte in bytes {
                    partial = partial.begin_list_item().map_err(reflect)?;
                    partial = partial.set(byte).map_err(reflect)?;
                    partial = partial.end().map_err(reflect)?;
                }
            } else {
                let mut entries = find_tree_entries(oid, store, depth, budget, expected)?;
                if validate_marker(&entries, store, depth, budget)? {
                    entries.clear();
                }
                sort_by_ordinal(&entries)?;
                for entry in entries {
                    partial = partial.begin_list_item().map_err(reflect)?;
                    partial = deser_into(
                        partial,
                        &entry.oid,
                        store,
                        depth + 1,
                        budget,
                        Some(entry.kind),
                        0,
                    )?;
                    partial = partial.end().map_err(reflect)?;
                }
            }
            return partial.end().map_err(reflect);
        }
        partial = deser_into(
            partial,
            oid,
            store,
            depth,
            budget,
            expected,
            wrapper_depth + 1,
        )?;
        return partial.end().map_err(reflect);
    }
    if shape.inner.is_some() && shape.vtable.has_try_borrow_inner() {
        let partial = partial.begin_inner().map_err(reflect)?;
        let partial = deser_into(
            partial,
            oid,
            store,
            depth,
            budget,
            expected,
            wrapper_depth + 1,
        )?;
        return partial.end().map_err(reflect);
    }

    if shape.scalar_type() == Some(ScalarType::Unit) {
        let entries = find_tree_entries(oid, store, depth, budget, expected)?;
        expect_marker(&entries, store, depth, budget)?;
        return partial.set(()).map_err(reflect);
    }

    match classify(shape) {
        ShapeClass::RawTree => {
            let mut buf = Vec::new();
            let data = find_object_with_budget(oid, &mut buf, store, depth, budget, expected)?;
            if data.kind != Kind::Tree {
                return Err(DeserializeError::NotATree(*oid));
            }
            partial.set(RawTree::new(*oid)).map_err(reflect)
        }
        ShapeClass::RawBlob => {
            let mut buf = Vec::new();
            let data = find_object_with_budget(oid, &mut buf, store, depth, budget, expected)?;
            if data.kind != Kind::Blob {
                return Err(DeserializeError::NotABlob(*oid));
            }
            partial.set(RawBlob::new(*oid)).map_err(reflect)
        }
        ShapeClass::Dynamic => deser_dynamic(partial, oid, store, depth, budget, expected),
        ShapeClass::Scalar => {
            let bytes = find_blob_bytes(oid, store, depth, budget, expected)?;
            parse_scalar(partial, oid, &bytes)
        }
        ShapeClass::Bytes => {
            let bytes = find_blob_raw(oid, store, depth, budget, expected)?;
            if shape.is_type::<Vec<u8>>() {
                return partial.set(bytes).map_err(reflect);
            }
            if let Def::Array(array) = shape.def {
                check_length(array.n, bytes.len())?;
                let mut partial = partial.init_array().map_err(reflect)?;
                for (index, byte) in bytes.into_iter().enumerate() {
                    partial = partial.begin_nth_field(index).map_err(reflect)?;
                    partial = partial.set(byte).map_err(reflect)?;
                    partial = partial.end().map_err(reflect)?;
                }
                return Ok(partial);
            }
            let mut partial = partial.init_list().map_err(reflect)?;
            for byte in bytes {
                partial = partial.begin_list_item().map_err(reflect)?;
                partial = partial.set(byte).map_err(reflect)?;
                partial = partial.end().map_err(reflect)?;
            }
            Ok(partial)
        }
        ShapeClass::Struct => {
            let facet::Type::User(facet::UserType::Struct(st)) = shape.ty else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            let positional = matches!(
                st.kind,
                facet::StructKind::Tuple | facet::StructKind::TupleStruct
            );
            let entries = find_tree_entries(oid, store, depth, budget, expected)?;
            deser_fields(
                partial, entries, st.fields, positional, store, depth, budget,
            )
        }
        ShapeClass::Sequence => {
            let mut entries = find_tree_entries(oid, store, depth, budget, expected)?;
            if validate_marker(&entries, store, depth, budget)? {
                entries.clear();
            }
            sort_by_ordinal(&entries)?;
            match shape.def {
                Def::List(_) => {
                    let mut partial = partial.init_list().map_err(reflect)?;
                    for entry in entries {
                        partial = partial.begin_list_item().map_err(reflect)?;
                        partial = deser_into(
                            partial,
                            &entry.oid,
                            store,
                            depth + 1,
                            budget,
                            Some(entry.kind),
                            0,
                        )?;
                        partial = partial.end().map_err(reflect)?;
                    }
                    Ok(partial)
                }
                Def::Array(array) => {
                    if array.n > MAX_TREE_ENTRIES {
                        return Err(DeserializeError::Unsupported(shape.type_identifier));
                    }
                    check_length(array.n, entries.len())?;
                    let mut partial = partial.init_array().map_err(reflect)?;
                    for (index, entry) in entries.into_iter().enumerate() {
                        partial = partial.begin_nth_field(index).map_err(reflect)?;
                        partial = deser_into(
                            partial,
                            &entry.oid,
                            store,
                            depth + 1,
                            budget,
                            Some(entry.kind),
                            0,
                        )?;
                        partial = partial.end().map_err(reflect)?;
                    }
                    Ok(partial)
                }
                _ => Err(DeserializeError::Unsupported(shape.type_identifier)),
            }
        }
        ShapeClass::Map => {
            let Def::Map(map) = shape.def else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            if map_uses_cbor(map)? {
                let bytes = find_blob_raw(oid, store, depth, budget, expected)?;
                let partial = crate::cbor::decode_into(partial, &bytes).map_err(|error| {
                    reflect(format!(
                        "invalid CBOR blob {oid} for {}: {error}",
                        shape.type_identifier
                    ))
                })?;
                if !budget.cbor_blobs.contains_key(oid) {
                    let actual = gix_object::compute_hash(oid.kind(), Kind::Blob, &bytes)
                        .map_err(reflect)?;
                    if actual != *oid {
                        return Err(reflect(format!("CBOR blob {oid} has incorrect content")));
                    }
                }
                *budget.cbor_blobs.entry(*oid).or_default() += 1;
                budget.cbor_hash_kinds.insert(oid.kind());
                return Ok(partial);
            }
            let mut entries = find_tree_entries(oid, store, depth, budget, expected)?;
            if validate_marker(&entries, store, depth, budget)? {
                entries.clear();
            }
            let mut partial = partial.init_map().map_err(reflect)?;
            for entry in entries {
                let key = user_name(&entry.name)?;
                partial = partial.begin_key().map_err(reflect)?;
                partial = parse_key_from_str(partial, key, 0)?;
                partial = partial.end().map_err(reflect)?;
                partial = partial.begin_value().map_err(reflect)?;
                partial = deser_into(
                    partial,
                    &entry.oid,
                    store,
                    depth + 1,
                    budget,
                    Some(entry.kind),
                    0,
                )?;
                partial = partial.end().map_err(reflect)?;
            }
            Ok(partial)
        }
        ShapeClass::Option => {
            let entries = find_tree_entries(oid, store, depth, budget, expected)?;
            let Some((inner_oid, kind)) = validate_option_entries(&entries, store, depth, budget)?
            else {
                return partial.set_default().map_err(reflect);
            };
            let partial = partial.begin_some().map_err(reflect)?;
            let partial = deser_into(partial, &inner_oid, store, depth + 1, budget, Some(kind), 0)?;
            partial.end().map_err(reflect)
        }
        ShapeClass::Enum => {
            let facet::Type::User(facet::UserType::Enum(enumeration)) = shape.ty else {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            };
            if enumeration.variants.len() > MAX_TREE_ENTRIES {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            }
            let (name, payload) = extract_enum_entry(oid, store, depth, budget, expected)?;
            let mut matches = enumeration
                .variants
                .iter()
                .enumerate()
                .filter(|(_, variant)| variant.effective_name() == name);
            let Some((index, variant)) = matches.next() else {
                return Err(reflect(format!(
                    "unknown enum variant {name:?} in {}",
                    shape.type_identifier
                )));
            };
            if matches.next().is_some() {
                return Err(reflect(format!("duplicate enum variant name {name:?}")));
            }
            let is_unit = matches!(variant.data.kind, facet::StructKind::Unit);
            let positional = matches!(
                variant.data.kind,
                facet::StructKind::Tuple | facet::StructKind::TupleStruct
            );
            let fields = variant.data.fields;
            if fields.len() > MAX_TREE_ENTRIES {
                return Err(DeserializeError::Unsupported(shape.type_identifier));
            }
            let mut partial = partial.select_nth_variant(index).map_err(reflect)?;
            let (inner_oid, kind) = match (is_unit, payload) {
                (true, None) => return Ok(partial),
                (true, Some(_)) => {
                    return Err(DeserializeError::UnitVariantIsTree { variant: name });
                }
                (false, Some(payload)) => payload,
                (false, None) => {
                    return Err(DeserializeError::VariantPayloadIsBlob { variant: name });
                }
            };
            if positional && fields.len() == 1 {
                partial = partial.begin_nth_field(0).map_err(reflect)?;
                partial = deser_into(partial, &inner_oid, store, depth + 1, budget, Some(kind), 0)?;
                return partial.end().map_err(reflect);
            }
            let entries = find_tree_entries(&inner_oid, store, depth + 1, budget, Some(kind))?;
            deser_fields(
                partial,
                entries,
                fields,
                positional,
                store,
                depth + 1,
                budget,
            )
        }
        ShapeClass::TransparentPointer
        | ShapeClass::TransparentNewtype
        | ShapeClass::Unsupported => Err(DeserializeError::Unsupported(shape.type_identifier)),
    }
}

/// Decode an untagged dynamic value by a deterministic, lossy heuristic.
fn deser_dynamic<'facet, F: Find + ?Sized>(
    partial: Partial<'facet, true>,
    oid: &ObjectId,
    store: &F,
    depth: usize,
    budget: &mut DecodeBudget,
    expected: Option<LeafKind>,
) -> Result<Partial<'facet, true>, DeserializeError> {
    let mut buf = Vec::new();
    let data = find_object_with_budget(oid, &mut buf, store, depth, budget, expected)?;
    if data.kind == Kind::Blob {
        // Dynamic bytes, like dynamic text, carry a trailing newline.
        let contents = data
            .data
            .strip_suffix(b"\n")
            .ok_or(DeserializeError::MissingLeafNewline(*oid))?;
        return match std::str::from_utf8(contents) {
            Ok(text) => partial.set(text.to_owned()).map_err(reflect),
            Err(_error) => partial.set(contents.to_owned()).map_err(reflect),
        };
    }
    let entries = tree_entries_from_data(&data, oid)?;
    if validate_marker(&entries, store, depth, budget)? {
        if partial.shape().is_type::<facet_value::Value>() {
            return partial.set(facet_value::Value::NULL).map_err(reflect);
        }
        return Err(DeserializeError::Unsupported(
            partial.shape().type_identifier,
        ));
    }
    if ordinal::validate_sequence(entries.iter().map(|entry| entry.name.encoded().as_bytes()))
        .is_ok()
    {
        let mut partial = partial.init_list().map_err(reflect)?;
        for entry in entries {
            partial = partial.begin_list_item().map_err(reflect)?;
            partial = deser_into(
                partial,
                &entry.oid,
                store,
                depth + 1,
                budget,
                Some(entry.kind),
                0,
            )?;
            partial = partial.end().map_err(reflect)?;
        }
        return Ok(partial);
    }
    let mut partial = partial.init_map().map_err(reflect)?;
    for entry in entries {
        let name = user_name(&entry.name)?;
        partial = partial.begin_object_entry(name).map_err(reflect)?;
        partial = deser_into(
            partial,
            &entry.oid,
            store,
            depth + 1,
            budget,
            Some(entry.kind),
            0,
        )?;
        partial = partial.end().map_err(reflect)?;
    }
    Ok(partial)
}

#[cfg(test)]
mod cbor_integration_tests {
    use std::collections::{BTreeMap, HashMap};
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use facet::Facet;
    use gix_object::{Kind, Write};
    use proptest::prelude::*;

    use crate::{
        DeserializeError, EntryKind, EntryMode, ObjectId, ObjectStore, TreeEntry, deserialize,
        serialize, serialize_into,
    };

    #[derive(Debug, Facet, PartialEq, Eq)]
    struct Document {
        maps: Vec<Option<BTreeMap<i32, String>>>,
        string_map: BTreeMap<String, u32>,
    }

    #[derive(Debug, Facet, PartialEq, Eq, PartialOrd, Ord)]
    struct CompoundKey {
        label: String,
        number: i32,
    }

    #[derive(Debug, Facet, PartialEq, Eq)]
    #[repr(u8)]
    enum Choice {
        Empty,
        Record { text: String, bytes: Vec<u8> },
        Pair(i64, bool),
    }

    #[derive(Debug, Facet, PartialEq, Eq)]
    #[repr(u8)]
    enum WrappedMap {
        Direct(BTreeMap<u8, u8>),
        Pair(bool, BTreeMap<u8, u8>),
        Named { inner: Option<BTreeMap<u8, u8>> },
    }

    #[derive(Debug, Facet, PartialEq, Eq)]
    struct NonString {
        data: BTreeMap<i32, String>,
    }

    #[derive(Debug, Facet, PartialEq, Eq)]
    struct RecursiveMap {
        children: Vec<RecursiveMap>,
        data: BTreeMap<i32, String>,
    }

    #[derive(Debug, Facet, PartialEq, Eq)]
    struct MapAndRawTree {
        data: BTreeMap<i32, String>,
        raw: crate::RawTree,
    }

    #[derive(Debug, Facet, PartialEq, Eq, PartialOrd, Ord)]
    #[facet(transparent)]
    struct Name(String);

    #[derive(Debug, Clone, Facet, PartialEq)]
    struct EqualFloat(f64);

    impl Eq for EqualFloat {}

    impl Hash for EqualFloat {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.to_bits().hash(state);
        }
    }

    fn write_blob(store: &ObjectStore, bytes: &[u8]) -> ObjectId {
        store.write_buf(Kind::Blob, bytes).expect("write blob")
    }

    fn write_tree(store: &ObjectStore, entries: Vec<(&str, EntryKind, ObjectId)>) -> ObjectId {
        let mut entries: Vec<TreeEntry> = entries
            .into_iter()
            .map(|(name, kind, oid)| TreeEntry {
                mode: EntryMode::from(kind),
                filename: name.into(),
                oid,
            })
            .collect();
        entries.sort();
        store
            .write(&gix_object::Tree { entries })
            .expect("write tree")
    }

    fn check_fixed_point<T>(value: &T)
    where
        T: for<'a> Facet<'a> + PartialEq + std::fmt::Debug,
    {
        for hash in [gix_hash::Kind::Sha1, gix_hash::Kind::Sha256] {
            let store = ObjectStore::with_hash_kind(hash);
            let root = serialize_into(value, &store).expect("serialize");
            let result: T = deserialize(&root, &store).expect("deserialize");
            assert_eq!(&result, value);
            assert_eq!(serialize_into(&result, &store).expect("re-encode"), root);
        }
    }

    #[test]
    fn integer_maps_are_raw_cbor_with_canonical_key_order() {
        let value = BTreeMap::from([
            (100i32, "x".to_owned()),
            (-1, "".to_owned()),
            (10, "z".to_owned()),
        ]);
        let (root, store) = serialize(&value).expect("serialize");
        assert_eq!(
            store.get_blob(&root).expect("read blob"),
            Some(vec![
                0xa3, 0x0a, 0x61, b'z', 0x18, 0x64, 0x61, b'x', 0x20, 0x60
            ])
        );
        let decoded: BTreeMap<i32, String> = deserialize(&root, &store).expect("decode");
        assert_eq!(decoded, value);
        assert_eq!(serialize(&decoded).expect("re-encode").0, root);
        check_fixed_point(&value);

        let empty = BTreeMap::<i32, String>::new();
        let (root, store) = serialize(&empty).expect("serialize empty");
        assert_eq!(store.get_blob(&root).expect("read blob"), Some(vec![0xa0]));
        check_fixed_point(&empty);
    }

    #[test]
    fn string_maps_and_string_like_keys_remain_trees() {
        let value = BTreeMap::from([("some".to_owned(), 42u32)]);
        let (root, store) = serialize(&value).expect("serialize");
        let entries = store.get_tree(&root).expect("read tree").expect("present");
        assert_eq!(entries.len(), 1);
        assert_eq!(AsRef::<[u8]>::as_ref(&entries[0].filename), b"%some");
        assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
        assert_eq!(
            store.get_blob(&entries[0].oid).expect("read blob"),
            Some(b"42\n".to_vec())
        );
        check_fixed_point(&value);
        check_fixed_point(&BTreeMap::from([(Arc::<str>::from("name"), 17u32)]));
        check_fixed_point(&BTreeMap::from([(Name("name".to_owned()), 17u32)]));
    }

    #[test]
    fn nested_compound_and_repeated_cbor_maps_are_verified() {
        let value = Document {
            maps: vec![
                Some(BTreeMap::from([(3, "three".to_owned())])),
                None,
                Some(BTreeMap::new()),
                Some(BTreeMap::from([(3, "three".to_owned())])),
            ],
            string_map: BTreeMap::from([("x".to_owned(), 9)]),
        };
        let (root, store) = serialize(&value).expect("serialize");
        let fields = store.get_tree(&root).expect("read root").expect("present");
        let map_field = fields
            .iter()
            .find(|field| AsRef::<[u8]>::as_ref(&field.filename) == b"maps")
            .unwrap();
        let options = store
            .get_tree(&map_field.oid)
            .expect("read list")
            .expect("present");
        for option in [0, 2, 3] {
            let entries = store
                .get_tree(&options[option].oid)
                .expect("read option")
                .expect("present");
            assert_eq!(AsRef::<[u8]>::as_ref(&entries[0].filename), b"some");
            assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
        }
        check_fixed_point(&value);
        check_fixed_point(&BTreeMap::from([
            (
                CompoundKey {
                    label: "alpha".to_owned(),
                    number: -2,
                },
                Choice::Record {
                    text: "x".to_owned(),
                    bytes: vec![0, 255],
                },
            ),
            (
                CompoundKey {
                    label: "beta".to_owned(),
                    number: 3,
                },
                Choice::Pair(17, true),
            ),
        ]));
        check_fixed_point(&BTreeMap::from([(7u32, Choice::Empty)]));
        check_fixed_point(&BTreeMap::from([(
            "outer".to_owned(),
            BTreeMap::from([(2i32, 1u8)]),
        )]));
        check_fixed_point(&Box::new(BTreeMap::from([(2i32, 1u8)])));
        for wrapped in [
            WrappedMap::Direct(BTreeMap::from([(1, 2)])),
            WrappedMap::Pair(true, BTreeMap::from([(3, 4)])),
            WrappedMap::Named {
                inner: Some(BTreeMap::from([(5, 6)])),
            },
            WrappedMap::Named { inner: None },
        ] {
            check_fixed_point(&wrapped);
        }
        check_fixed_point(&RecursiveMap {
            children: vec![RecursiveMap {
                children: vec![],
                data: BTreeMap::from([(1, "one".to_owned())]),
            }],
            data: BTreeMap::from([(2, "two".to_owned())]),
        });
        for hash in [gix_hash::Kind::Sha1, gix_hash::Kind::Sha256] {
            let store = ObjectStore::with_hash_kind(hash);
            let raw = write_tree(&store, vec![]);
            let value = MapAndRawTree {
                data: BTreeMap::from([(1, "one".to_owned())]),
                raw: crate::RawTree::new(raw),
            };
            let root = serialize_into(&value, &store).expect("serialize with raw tree");
            let decoded: MapAndRawTree = deserialize(&root, &store).expect("decode with raw tree");
            assert_eq!(decoded, value);
            assert_eq!(serialize_into(&decoded, &store).unwrap(), root);
        }
    }

    #[test]
    fn noncanonical_malformed_or_wrong_kind_maps_are_rejected() {
        let store = ObjectStore::default();
        for bytes in [
            &b""[..],
            &b"\xa0\n"[..],
            &b"\xbf\xff"[..],
            &b"\xb8\x00"[..],
            &b"\xa1\x18\x00\x61x"[..],
            &b"\xa2\x01\x61a\x01\x61b"[..],
            &b"\xa2\x02\x61a\x01\x61b"[..],
            &b"\xa1\x01"[..],
            &b"\xa1\x01\x61\xff"[..],
            &b"\x80"[..],
            &b"\xa1\x01\x02"[..],
            &b"\xa1\x1b\xff\xff\xff\xff\xff\xff\xff\xff\x61x"[..],
        ] {
            let root = write_blob(&store, bytes);
            assert!(
                deserialize::<BTreeMap<i32, String>>(&root, &store).is_err(),
                "accepted malformed map {bytes:?}"
            );
        }
        let empty = write_blob(&store, b"");
        let tree = write_tree(&store, vec![("none", EntryKind::Blob, empty)]);
        assert!(matches!(
            deserialize::<BTreeMap<i32, String>>(&tree, &store),
            Err(DeserializeError::NotABlob(_))
        ));
        let blob = write_blob(&store, &[0xa0]);
        assert!(matches!(
            deserialize::<BTreeMap<String, String>>(&blob, &store),
            Err(DeserializeError::NotATree(_))
        ));
        let wrong_mode = write_tree(&store, vec![("data", EntryKind::Tree, blob)]);
        assert!(matches!(
            deserialize::<NonString>(&wrong_mode, &store),
            Err(DeserializeError::NotATree(_))
        ));
    }

    #[test]
    fn rejects_collapsed_map_keys_inside_a_nested_cbor_value() {
        // +0.0 and -0.0 have different CBOR bytes but compare equal as Rust keys.
        let bytes = [
            0xa2, 0x81, 0xf9, 0x00, 0x00, 0x01, 0x81, 0xf9, 0x80, 0x00, 0x02,
        ];
        let store = ObjectStore::default();
        let blob = write_blob(&store, &bytes);
        assert!(deserialize::<HashMap<EqualFloat, u8>>(&blob, &store).is_err());
        let root = write_tree(&store, vec![("data", EntryKind::Blob, blob)]);
        #[derive(Facet)]
        struct Nested {
            data: HashMap<EqualFloat, u8>,
        }
        assert!(deserialize::<Nested>(&root, &store).is_err());
    }

    #[test]
    fn non_string_maps_are_not_subject_to_tree_entry_cap() {
        let value: BTreeMap<u16, u16> = (0..=4096).map(|n| (n, n)).collect();
        let (root, store) = serialize(&value).expect("encode CBOR map");
        assert!(store.get_blob(&root).expect("read").is_some());
        let result: BTreeMap<u16, u16> = deserialize(&root, &store).expect("decode");
        assert_eq!(result, value);
        assert_eq!(serialize(&result).expect("re-encode").0, root);
    }

    proptest! {
        #[test]
        fn integer_maps_roundtrip_and_preserve_blob(
            pairs in proptest::collection::btree_map(any::<i16>(), any::<String>(), 0..32)
        ) {
            let (root, store) = serialize(&pairs).expect("encode");
            let decoded: BTreeMap<i16, String> = deserialize(&root, &store).expect("decode");
            prop_assert_eq!(&decoded, &pairs);
            prop_assert_eq!(serialize(&decoded).expect("re-encode").0, root);
        }

        #[test]
        fn arbitrary_accepted_cbor_map_reproduces_its_blob(
            bytes in proptest::collection::vec(any::<u8>(), 0..128)
        ) {
            let store = ObjectStore::default();
            let root = write_blob(&store, &bytes);
            if let Ok(value) = deserialize::<BTreeMap<i16, String>>(&root, &store) {
                prop_assert_eq!(serialize(&value).expect("re-encode").0, root);
            }
        }
    }
}
