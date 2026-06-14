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
    /// A facet key collides with a reserved sentinel name (`.schema`, `.variant`).
    ///
    /// These names carry facet-git-tree's own bookkeeping, so user data may not
    /// use them as keys; [`check_key`] enforces this on the write side.
    #[error("reserved key {0:?}: collides with a sentinel name")]
    ReservedKey(String),
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

/// Tree entry names reserved by the encoding for its own bookkeeping.
///
/// `.schema` carries the type definition attached to every schema object and
/// `.variant` the selected enum variant. User data may not use these as keys;
/// [`check_key`] rejects them.
const RESERVED_NAMES: [&str; 2] = [".schema", ".variant"];

/// Validate a user-supplied key for use as a Git tree entry name.
///
/// Keys become tree entry names, which double as path segments, so a key may
/// neither contain the path separator `/` ([`Error::InvalidKey`]) nor collide
/// with a reserved sentinel name — `.schema` or `.variant` ([`Error::ReservedKey`]).
/// Serialization is required to apply this to every dynamic key (such as map
/// keys) before emitting its entry, so reserved names can never be written as data.
pub fn check_key(key: &str) -> Result<(), Error> {
    if key.contains('/') {
        return Err(Error::InvalidKey(key.to_owned()));
    }
    if RESERVED_NAMES.contains(&key) {
        return Err(Error::ReservedKey(key.to_owned()));
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
    // The implementation MUST validate every dynamic key (e.g. map keys) with
    // [`check_key`] before emitting its tree entry, so that reserved sentinel
    // names (`.schema`, `.variant`) and `/`-bearing names can never be written
    // as data. Static field names come from Rust identifiers and are safe.
    let _ = (value, store);
    todo!("serialization not yet implemented")
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
    let _ = (root, store);
    todo!("deserialization not yet implemented")
}
