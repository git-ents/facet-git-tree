//! Serialize [`facet::Facet`] values to, and deserialize them from, Git trees.
//!
//! A value is encoded as a graph of Git objects — scalars and strings as blobs,
//! structs, enums, and collections as trees — addressed by SHA-1 exactly as Git
//! would compute them. The bundled [`ObjectStore`] is an in-memory backend, but
//! the entry points are generic over `gix`'s `Find` and `Write` traits, so a
//! real `gix` repository or object database works just as well.
//!
//! The normative encoding rules live in `docs/specification.adoc`.
#![forbid(unsafe_code)]

use std::io::Read;

pub use gix_hash::ObjectId;
pub use gix_object::Object as GitObject;
pub use gix_object::tree::{Entry as TreeEntry, EntryKind, EntryMode};

use gix_hash::Kind as HashKind;
use gix_object::{Data, Find, Kind, ObjectRef, Write};

use facet::Def;
use facet::{Partial, Peek};

/// A content-addressed store of Git objects produced by [`serialize`].
///
/// This is a thin wrapper around [`gix_odb::memory::Proxy`], gitoxide's own
/// in-memory object database, so the `Find`/`Write` buffer handling lives in
/// `gix` rather than being reimplemented here. The accessors return owned values
/// because the proxy is `RefCell`-backed (required since [`Write::write_stream`]
/// takes `&self`).
///
/// This type is only a convenience default for callers that lack a backend of
/// their own. The actual contract is the generic `gix` [`Find`]/[`Write`] bounds
/// on [`serialize_into`] and [`deserialize`], which a real `gix` repository or
/// odb satisfies just as well. Like `gix`'s in-memory store it is `!Sync`;
/// cross-thread sharing is the job of the on-disk backends, not of this type.
pub struct ObjectStore(gix_odb::memory::Proxy<NoBackend>);

impl Default for ObjectStore {
    fn default() -> Self {
        Self(gix_odb::memory::Proxy::new(NoBackend, HashKind::Sha1))
    }
}

impl std::fmt::Debug for ObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStore")
            .field("objects", &self.0.num_objects_in_memory())
            .finish()
    }
}

impl ObjectStore {
    /// Decode and return the object stored under `id`, if present.
    pub fn get(&self, id: &ObjectId) -> Option<GitObject> {
        let mut buf = Vec::new();
        let data = self.0.try_find(id, &mut buf).ok().flatten()?;
        ObjectRef::from_bytes(data.data, data.kind, HashKind::Sha1)
            .ok()?
            .into_owned()
            .ok()
    }

    /// Return the entries of the tree stored under `id`, if it is a tree.
    pub fn get_tree(&self, id: &ObjectId) -> Option<Vec<TreeEntry>> {
        match self.get(id)? {
            GitObject::Tree(tree) => Some(tree.entries),
            _ => None,
        }
    }

    /// Return the raw bytes of the blob stored under `id`, if it is a blob.
    pub fn get_blob(&self, id: &ObjectId) -> Option<Vec<u8>> {
        match self.get(id)? {
            GitObject::Blob(blob) => Some(blob.data),
            _ => None,
        }
    }
}

impl Find for ObjectStore {
    fn try_find<'a>(
        &self,
        id: &gix_hash::oid,
        buffer: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
        self.0.try_find(id, buffer)
    }
}

impl Write for ObjectStore {
    fn write_stream(
        &self,
        kind: Kind,
        size: u64,
        from: &mut dyn Read,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.0.write_stream(kind, size, from)
    }
}

/// Inert backing database for [`ObjectStore`]'s in-memory [`Proxy`].
///
/// [`gix_odb::memory::Proxy`] is generic over an inner object database it falls
/// back to, but [`ObjectStore`] keeps everything in the proxy's in-memory map, so
/// the inner is never read from or written to. gitoxide ships no type that is
/// both [`Find`] and [`Write`] while doing nothing, so this supplies one.
#[derive(Debug, Default)]
struct NoBackend;

impl Find for NoBackend {
    fn try_find<'a>(
        &self,
        _id: &gix_hash::oid,
        _buffer: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
        Ok(None)
    }
}

impl Write for NoBackend {
    fn write_stream(
        &self,
        _kind: Kind,
        _size: u64,
        _from: &mut dyn Read,
    ) -> Result<ObjectId, gix_object::write::Error> {
        // The enclosing `Proxy` always has its in-memory store enabled, so writes
        // are intercepted before reaching this inner database.
        Err("NoBackend: writes are handled by the in-memory proxy".into())
    }
}

/// An error produced by serialization or deserialization.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A facet key cannot be represented as a Git tree entry name.
    ///
    /// Tree entry names double as path segments, so a key may not contain the
    /// path separator `/`.
    #[error("invalid key {0:?}: must not contain '/'")]
    InvalidKey(String),
    /// A tree entry name (path segment) is not valid UTF-8.
    ///
    /// Holds the lossily-decoded name for diagnostics. Write-side names are
    /// always UTF-8, so this can only arise from an externally-produced tree.
    #[error("tree entry name {0:?} is not valid UTF-8")]
    NonUtf8Name(String),
    /// A referenced object was not present in its backing store.
    #[error("object {0} not found")]
    NotFound(ObjectId),
    /// An object was expected to be a tree but was of another kind.
    #[error("object {0} is not a tree")]
    NotATree(ObjectId),
    /// An error from the underlying `gix` object backend.
    ///
    /// Wraps the backend's own error (from [`Find`]/[`Write`]) as the source
    /// rather than flattening it into a string.
    #[error("git object backend error")]
    Backend(#[source] gix_object::write::Error),
    /// A general serialization or deserialization failure.
    #[error("{0}")]
    Message(String),
}

/// Validate a user-supplied key for use as a Git tree entry name.
///
/// Keys become tree entry names, which double as path segments, so a key may not
/// contain the path separator `/` ([`Error::InvalidKey`]). Serialization is
/// required to apply this to every dynamic key (such as map keys) before emitting
/// its entry, so a `/`-bearing name can never be written as data.
pub fn check_key(key: &str) -> Result<(), Error> {
    if key.contains('/') {
        return Err(Error::InvalidKey(key.to_owned()));
    }
    Ok(())
}

/// Serialize a [`facet::Facet`] value into the given `gix` object `store`.
///
/// Writes all blobs and sub-trees reachable from `value` and returns the root
/// tree [`ObjectId`]. This is the generic core; [`serialize`] is a convenience
/// wrapper that allocates a fresh [`ObjectStore`].
///
/// `store` is the backend contract: any `gix` [`Write`] sink works — a real
/// `gix` repository, an in-memory odb proxy, or the bundled [`ObjectStore`]. The
/// bound is `&self` (never `&mut`) because `gix`'s `Write` is; that is what lets
/// one backend be shared while objects stream into it. `?Sized` is permitted so
/// a `&dyn Write` may be passed for runtime backend selection.
pub fn serialize_into<T, W>(value: &T, store: &W) -> Result<ObjectId, Error>
where
    T: for<'a> facet::Facet<'a>,
    W: Write + ?Sized,
{
    let peek = Peek::new(value);
    let (oid, _kind) = serialize_peek(peek, store)?;
    Ok(oid)
}

/// Serialize a [`facet::Facet`] value into a set of Git objects.
///
/// Returns the root [`ObjectId`] (a tree) and an [`ObjectStore`] containing
/// all blobs and sub-trees reachable from that root.
pub fn serialize<T: for<'a> facet::Facet<'a>>(value: &T) -> Result<(ObjectId, ObjectStore), Error> {
    let store = ObjectStore::default();
    let root = serialize_into(value, &store)?;
    Ok((root, store))
}

/// Deserialize a [`facet::Facet`] value from a root tree stored in `store`.
///
/// `store` is any `gix` [`Find`] source — a real repository, an in-memory odb,
/// or an [`ObjectStore`] — the read side of the backend contract documented on
/// [`serialize_into`]. `?Sized` is permitted so a `&dyn Find` may be passed.
pub fn deserialize<T: for<'a> facet::Facet<'a>>(
    root: &ObjectId,
    store: &(impl Find + ?Sized),
) -> Result<T, Error> {
    let partial =
        Partial::alloc::<T>().map_err(|e| Error::Message(format!("alloc failed: {e}")))?;
    let partial = deser_into(partial, root, store)?;
    let heap = partial
        .build()
        .map_err(|e| Error::Message(format!("build failed: {e}")))?;
    heap.materialize::<T>()
        .map_err(|e| Error::Message(format!("materialize failed: {e}")))
}

// --- serialization internals ---

fn serialize_peek<W: Write + ?Sized>(
    peek: Peek<'_, '_>,
    store: &W,
) -> Result<(ObjectId, EntryKind), Error> {
    let shape = peek.shape();

    // Scalar leaf → blob
    if matches!(shape.def, Def::Scalar) {
        let bytes = scalar_bytes(peek)?;
        let oid = store
            .write_buf(Kind::Blob, &bytes)
            .map_err(Error::Backend)?;
        return Ok((oid, EntryKind::Blob));
    }

    // Struct or tuple → tree. A named struct keys entries by field name; a tuple
    // or tuple struct keys them by zero-padded positional ordinal (facet models
    // all of these as `UserType::Struct`, distinguished by `StructKind`).
    if let facet::Type::User(facet::UserType::Struct(st)) = shape.ty {
        let positional = matches!(
            st.kind,
            facet::StructKind::Tuple | facet::StructKind::TupleStruct
        );
        let ps = peek
            .into_struct()
            .map_err(|e| Error::Message(e.to_string()))?;
        let mut entries: Vec<TreeEntry> = Vec::with_capacity(st.fields.len());
        for (i, field) in st.fields.iter().enumerate() {
            let child = ps.field(i).map_err(|e| Error::Message(e.to_string()))?;
            let (oid, kind) = serialize_peek(child, store)?;
            let filename: gix_object::bstr::BString = if positional {
                format!("{i:04}").into()
            } else {
                field.name.into()
            };
            entries.push(TreeEntry {
                mode: EntryMode::from(kind),
                filename,
                oid,
            });
        }
        entries.sort();
        let oid = store
            .write(&gix_object::Tree { entries })
            .map_err(Error::Backend)?;
        return Ok((oid, EntryKind::Tree));
    }

    // Vec / Array / slice → tree with ordinal keys
    if matches!(shape.def, Def::List(_) | Def::Array(_)) {
        let entries = serialize_sequence(peek, store)?;
        let oid = store
            .write(&gix_object::Tree { entries })
            .map_err(Error::Backend)?;
        return Ok((oid, EntryKind::Tree));
    }

    // Map → tree keyed by map keys
    if matches!(shape.def, Def::Map(_)) {
        let pm = peek.into_map().map_err(|e| Error::Message(e.to_string()))?;
        let mut entries: Vec<TreeEntry> = Vec::new();
        for (k, v) in pm.iter() {
            let key_str = k
                .as_str()
                .ok_or_else(|| Error::Message("map key must be a string".into()))?;
            check_key(key_str)?;
            let (oid, kind) = serialize_peek(v, store)?;
            entries.push(TreeEntry {
                mode: EntryMode::from(kind),
                filename: key_str.into(),
                oid,
            });
        }
        entries.sort();
        let oid = store
            .write(&gix_object::Tree { entries })
            .map_err(Error::Backend)?;
        return Ok((oid, EntryKind::Tree));
    }

    // Option
    if matches!(shape.def, Def::Option(_)) {
        let po = peek
            .into_option()
            .map_err(|e| Error::Message(e.to_string()))?;
        if let Some(inner) = po.value() {
            let (oid, kind) = serialize_peek(inner, store)?;
            // Some: wrap in a tree with a single "some" entry
            let entries = vec![TreeEntry {
                mode: EntryMode::from(kind),
                filename: "some".into(),
                oid,
            }];
            let oid = store
                .write(&gix_object::Tree { entries })
                .map_err(Error::Backend)?;
            return Ok((oid, EntryKind::Tree));
        } else {
            // None: empty tree
            let oid = store
                .write(&gix_object::Tree { entries: vec![] })
                .map_err(Error::Backend)?;
            return Ok((oid, EntryKind::Tree));
        }
    }

    // Enum → single-entry tree: variant name → variant contents
    if let facet::Type::User(facet::UserType::Enum(_)) = shape.ty {
        let pe = peek
            .into_enum()
            .map_err(|e| Error::Message(e.to_string()))?;
        let variant = pe
            .active_variant()
            .map_err(|e| Error::Message(e.to_string()))?;
        let variant_name = pe
            .variant_name_active()
            .map_err(|e| Error::Message(e.to_string()))?;

        // Encode the variant's payload (unit → empty tree, newtype → the field's
        // own encoding directly, tuple → ordinal-keyed tree, struct → name-keyed
        // tree). A tuple variant is `StructKind::TupleStruct`; a struct variant is
        // `StructKind::Struct`.
        let positional = matches!(variant.data.kind, facet::StructKind::TupleStruct);
        let newtype = positional && variant.data.fields.len() == 1;
        let (inner_oid, inner_kind) = if variant.data.fields.is_empty() {
            let oid = store
                .write(&gix_object::Tree { entries: vec![] })
                .map_err(Error::Backend)?;
            (oid, EntryKind::Tree)
        } else if newtype {
            // Newtype variant: resolves directly to the encoding of its one field.
            let child = pe
                .field(0)
                .map_err(|e| Error::Message(e.to_string()))?
                .ok_or_else(|| Error::Message("variant field 0 missing".into()))?;
            serialize_peek(child, store)?
        } else {
            let mut inner_entries: Vec<TreeEntry> = Vec::new();
            for (i, field) in variant.data.fields.iter().enumerate() {
                let child = pe
                    .field(i)
                    .map_err(|e| Error::Message(e.to_string()))?
                    .ok_or_else(|| Error::Message(format!("variant field {i} missing")))?;
                let (oid, kind) = serialize_peek(child, store)?;
                let name: gix_object::bstr::BString = if positional {
                    format!("{i:04}").into()
                } else {
                    field.name.into()
                };
                inner_entries.push(TreeEntry {
                    mode: EntryMode::from(kind),
                    filename: name,
                    oid,
                });
            }
            inner_entries.sort();
            let oid = store
                .write(&gix_object::Tree {
                    entries: inner_entries,
                })
                .map_err(Error::Backend)?;
            (oid, EntryKind::Tree)
        };

        let entries = vec![TreeEntry {
            mode: EntryMode::from(inner_kind),
            filename: variant_name.into(),
            oid: inner_oid,
        }];
        let oid = store
            .write(&gix_object::Tree { entries })
            .map_err(Error::Backend)?;
        return Ok((oid, EntryKind::Tree));
    }

    Err(Error::Message(format!(
        "unsupported type for serialization: {}",
        shape.type_identifier
    )))
}

fn serialize_sequence<W: Write + ?Sized>(
    peek: Peek<'_, '_>,
    store: &W,
) -> Result<Vec<TreeEntry>, Error> {
    let shape = peek.shape();
    let mut entries: Vec<TreeEntry> = Vec::new();

    if matches!(shape.def, Def::List(_)) {
        let pl = peek
            .into_list()
            .map_err(|e| Error::Message(e.to_string()))?;
        for (i, item) in pl.iter().enumerate() {
            let (oid, kind) = serialize_peek(item, store)?;
            entries.push(TreeEntry {
                mode: EntryMode::from(kind),
                filename: format!("{i:04}").into(),
                oid,
            });
        }
    } else if matches!(shape.def, Def::Array(_)) {
        let pa = peek
            .into_list_like()
            .map_err(|e| Error::Message(e.to_string()))?;
        for (i, item) in pa.iter().enumerate() {
            let (oid, kind) = serialize_peek(item, store)?;
            entries.push(TreeEntry {
                mode: EntryMode::from(kind),
                filename: format!("{i:04}").into(),
                oid,
            });
        }
    }

    entries.sort();
    Ok(entries)
}

fn scalar_bytes(peek: Peek<'_, '_>) -> Result<Vec<u8>, Error> {
    // Strings: verbatim UTF-8 bytes
    if let Some(s) = peek.as_str() {
        return Ok(s.as_bytes().to_vec());
    }

    // Use Display for everything else, with special float/bool/char handling
    let shape = peek.shape();
    if let facet::Type::Primitive(pt) = shape.ty {
        use facet::{NumericType, PrimitiveType, TextualType};
        match pt {
            PrimitiveType::Boolean => {
                let v = *peek
                    .get::<bool>()
                    .map_err(|e| Error::Message(e.to_string()))?;
                return Ok(v.to_string().into_bytes());
            }
            PrimitiveType::Textual(TextualType::Char) => {
                let v = *peek
                    .get::<char>()
                    .map_err(|e| Error::Message(e.to_string()))?;
                let mut buf = [0u8; 4];
                return Ok(v.encode_utf8(&mut buf).as_bytes().to_vec());
            }
            PrimitiveType::Textual(TextualType::Str) => {
                // handled above by as_str(); shouldn't reach here
                if let Some(s) = peek.as_str() {
                    return Ok(s.as_bytes().to_vec());
                }
            }
            PrimitiveType::Numeric(NumericType::Float) => {
                let layout_size = shape.layout.sized_layout().map(|l| l.size()).unwrap_or(8);
                if layout_size == 4 {
                    let v = *peek
                        .get::<f32>()
                        .map_err(|e| Error::Message(e.to_string()))?;
                    if v.is_nan() {
                        return Ok(b"nan".to_vec());
                    }
                    let v = if v == 0.0f32 { 0.0f32 } else { v };
                    return Ok(v.to_string().into_bytes());
                } else {
                    let v = *peek
                        .get::<f64>()
                        .map_err(|e| Error::Message(e.to_string()))?;
                    if v.is_nan() {
                        return Ok(b"nan".to_vec());
                    }
                    let v = if v == 0.0f64 { 0.0f64 } else { v };
                    return Ok(v.to_string().into_bytes());
                }
            }
            PrimitiveType::Numeric(NumericType::Integer { signed }) => {
                let layout_size = shape.layout.sized_layout().map(|l| l.size()).unwrap_or(8);
                if signed {
                    match layout_size {
                        1 => {
                            return Ok(peek
                                .get::<i8>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        2 => {
                            return Ok(peek
                                .get::<i16>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        4 => {
                            return Ok(peek
                                .get::<i32>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        8 => {
                            return Ok(peek
                                .get::<i64>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        16 => {
                            return Ok(peek
                                .get::<i128>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        _ => {
                            return Ok(peek
                                .get::<isize>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                    }
                } else {
                    match layout_size {
                        1 => {
                            return Ok(peek
                                .get::<u8>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        2 => {
                            return Ok(peek
                                .get::<u16>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        4 => {
                            return Ok(peek
                                .get::<u32>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        8 => {
                            return Ok(peek
                                .get::<u64>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        16 => {
                            return Ok(peek
                                .get::<u128>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                        _ => {
                            return Ok(peek
                                .get::<usize>()
                                .map_err(|e| Error::Message(e.to_string()))?
                                .to_string()
                                .into_bytes());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Err(Error::Message(format!(
        "unsupported scalar type: {}",
        shape.type_identifier
    )))
}

// --- deserialization internals ---

fn find_object<'a, F: Find + ?Sized>(
    id: &ObjectId,
    buf: &'a mut Vec<u8>,
    store: &F,
) -> Result<Data<'a>, Error> {
    store
        .try_find(id, buf)
        .map_err(|e| Error::Message(e.to_string()))?
        .ok_or_else(|| Error::NotFound(*id))
}

fn find_tree_entries<F: Find + ?Sized>(
    id: &ObjectId,
    store: &F,
) -> Result<Vec<(String, ObjectId, EntryKind)>, Error> {
    let mut buf = Vec::new();
    let data = find_object(id, &mut buf, store)?;
    if data.kind != Kind::Tree {
        return Err(Error::NotATree(*id));
    }
    let tree_ref = gix_object::TreeRef::from_bytes(data.data, HashKind::Sha1)
        .map_err(|e| Error::Message(e.to_string()))?;
    let mut result = Vec::new();
    for entry in &tree_ref.entries {
        let name = std::str::from_utf8(entry.filename).map_err(|_| {
            Error::NonUtf8Name(String::from_utf8_lossy(entry.filename).into_owned())
        })?;
        result.push((name.to_owned(), entry.oid.to_owned(), entry.mode.kind()));
    }
    Ok(result)
}

fn find_blob_bytes<F: Find + ?Sized>(id: &ObjectId, store: &F) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    let data = find_object(id, &mut buf, store)?;
    Ok(data.data.to_owned())
}

fn deser_into<'facet, F: Find + ?Sized>(
    partial: Partial<'facet, true>,
    oid: &ObjectId,
    store: &F,
) -> Result<Partial<'facet, true>, Error> {
    let shape = partial.shape();

    // Scalar leaf: read blob, parse from str
    if matches!(shape.def, Def::Scalar) {
        let bytes = find_blob_bytes(oid, store)?;
        let s = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Message("blob is not valid UTF-8".into()))?;
        return partial
            .parse_from_str(s)
            .map_err(|e| Error::Message(format!("parse failed: {e}")));
    }

    // Struct: read tree, fill fields by name. Tuples and tuple structs key their
    // entries by zero-padded positional ordinal (mirroring serialization).
    if let facet::Type::User(facet::UserType::Struct(st)) = shape.ty {
        let positional = matches!(
            st.kind,
            facet::StructKind::Tuple | facet::StructKind::TupleStruct
        );
        let entries = find_tree_entries(oid, store)?;
        let mut partial = partial;
        for (i, field) in st.fields.iter().enumerate() {
            // Find this field's entry in the tree
            let entry_name = if positional {
                format!("{i:04}")
            } else {
                field.name.to_string()
            };
            let entry = entries.iter().find(|(name, _, _)| *name == entry_name);
            if let Some((_, child_oid, _)) = entry {
                let child_oid = *child_oid;
                partial = partial
                    .begin_field(field.name)
                    .map_err(|e| Error::Message(format!("begin_field {}: {e}", field.name)))?;
                partial = deser_into(partial, &child_oid, store)?;
                partial = partial
                    .end()
                    .map_err(|e| Error::Message(format!("end field {}: {e}", field.name)))?;
            }
        }
        return Ok(partial);
    }

    // List (Vec): read tree with ordinal keys, sort numerically, push items
    if matches!(shape.def, Def::List(_)) {
        let mut entries = find_tree_entries(oid, store)?;
        entries.sort_by_key(|(name, _, _)| name.parse::<usize>().unwrap_or(0));
        let mut partial = partial
            .init_list()
            .map_err(|e| Error::Message(e.to_string()))?;
        for (_, child_oid, _) in entries {
            partial = partial
                .begin_list_item()
                .map_err(|e| Error::Message(e.to_string()))?;
            partial = deser_into(partial, &child_oid, store)?;
            partial = partial.end().map_err(|e| Error::Message(e.to_string()))?;
        }
        return Ok(partial);
    }

    // Array: same as List but init_array
    if matches!(shape.def, Def::Array(_)) {
        let mut entries = find_tree_entries(oid, store)?;
        entries.sort_by_key(|(name, _, _)| name.parse::<usize>().unwrap_or(0));
        let mut partial = partial
            .init_array()
            .map_err(|e| Error::Message(e.to_string()))?;
        for (i, (_, child_oid, _)) in entries.into_iter().enumerate() {
            partial = partial
                .begin_nth_field(i)
                .map_err(|e| Error::Message(e.to_string()))?;
            partial = deser_into(partial, &child_oid, store)?;
            partial = partial.end().map_err(|e| Error::Message(e.to_string()))?;
        }
        return Ok(partial);
    }

    // Map
    if matches!(shape.def, Def::Map(_)) {
        let entries = find_tree_entries(oid, store)?;
        let mut partial = partial
            .init_map()
            .map_err(|e| Error::Message(e.to_string()))?;
        for (key, child_oid, _) in entries {
            partial = partial
                .begin_key()
                .map_err(|e| Error::Message(e.to_string()))?;
            partial = partial
                .parse_from_str(&key)
                .map_err(|e| Error::Message(e.to_string()))?;
            partial = partial.end().map_err(|e| Error::Message(e.to_string()))?;
            partial = partial
                .begin_value()
                .map_err(|e| Error::Message(e.to_string()))?;
            partial = deser_into(partial, &child_oid, store)?;
            partial = partial.end().map_err(|e| Error::Message(e.to_string()))?;
        }
        return Ok(partial);
    }

    // Option: empty tree → None, single-entry "some" tree → Some(inner)
    if matches!(shape.def, Def::Option(_)) {
        let entries = find_tree_entries(oid, store)?;
        if entries.is_empty() {
            // None — partial is already default None, just return
            return Ok(partial);
        } else {
            let (_, inner_oid, _) = &entries[0];
            let inner_oid = *inner_oid;
            let partial = partial
                .begin_some()
                .map_err(|e| Error::Message(e.to_string()))?;
            let partial = deser_into(partial, &inner_oid, store)?;
            return partial.end().map_err(|e| Error::Message(e.to_string()));
        }
    }

    // Enum: single-entry tree → variant name → variant contents
    if let facet::Type::User(facet::UserType::Enum(et)) = shape.ty {
        let entries = find_tree_entries(oid, store)?;
        let (variant_name, inner_oid, _) = entries
            .into_iter()
            .next()
            .ok_or_else(|| Error::Message("enum tree must have exactly one entry".into()))?;

        // The variant's field layout comes from the type, not the tree: a tuple
        // variant (`TupleStruct`) keys by ordinal, a struct variant by name, and a
        // newtype (single-field tuple) variant resolves directly to its field.
        let variant = et.variants.iter().find(|v| v.name == variant_name);
        let positional =
            variant.is_some_and(|v| matches!(v.data.kind, facet::StructKind::TupleStruct));
        let newtype = positional && variant.is_some_and(|v| v.data.fields.len() == 1);

        let mut partial = partial
            .select_variant_named(&variant_name)
            .map_err(|e| Error::Message(format!("select variant {variant_name}: {e}")))?;

        if newtype {
            partial = partial
                .begin_nth_field(0)
                .map_err(|e| Error::Message(e.to_string()))?;
            partial = deser_into(partial, &inner_oid, store)?;
            return partial.end().map_err(|e| Error::Message(e.to_string()));
        }

        let inner_entries = find_tree_entries(&inner_oid, store)?;
        for (name, child_oid, _) in inner_entries {
            if positional {
                let idx = name
                    .parse::<usize>()
                    .map_err(|_| Error::Message(format!("invalid ordinal: {name}")))?;
                partial = partial
                    .begin_nth_field(idx)
                    .map_err(|e| Error::Message(e.to_string()))?;
            } else {
                partial = partial
                    .begin_field(&name)
                    .map_err(|e| Error::Message(e.to_string()))?;
            }
            partial = deser_into(partial, &child_oid, store)?;
            partial = partial.end().map_err(|e| Error::Message(e.to_string()))?;
        }
        return Ok(partial);
    }

    Err(Error::Message(format!(
        "unsupported type for deserialization: {}",
        shape.type_identifier
    )))
}
