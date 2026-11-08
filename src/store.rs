//! The bundled in-memory [`ObjectStore`] backend.

use std::io::Read;
use std::marker::PhantomData;

use gix_object::{Data, Find, Kind, Write, WriteTo};

use crate::error::{DeserializeError, SerializeError};
use crate::hash::{HashFormat, Oid, Sha1, Tagged};
use crate::{GitObject, ObjectId, TreeEntry};

/// A content-addressed store of Git objects produced by
/// [`serialize`](crate::serialize), parameterised by its hash format.
///
/// This is a thin wrapper around [`gix_odb::memory::Proxy`], gitoxide's own
/// in-memory object database, so the `Find`/`Write` buffer handling lives in
/// `gix` rather than being reimplemented here. The accessors return owned values
/// because the proxy is `RefCell`-backed (required since [`Write::write_stream`]
/// takes `&self`).
///
/// The format parameter `F` is the hash format every id this store produces
/// uses; the typed accessors only accept an [`Oid<F>`], so an id of the other
/// format cannot be presented to the wrong store. [`ObjectStore::default`]
/// (and the bare `ObjectStore` mention) is SHA-1.
///
/// This type is only a convenience default for callers that lack a backend of
/// their own. The actual contract is the generic `gix` [`Find`]/[`Write`] bounds
/// on [`serialize_into`](crate::serialize_into) and
/// [`deserialize`](crate::deserialize), which a real `gix` repository or
/// odb satisfies just as well — wrapped in [`Backend`](crate::Backend) to claim
/// its format. Like `gix`'s in-memory store it is `!Sync`; cross-thread sharing
/// is the job of the on-disk backends, not of this type.
pub struct ObjectStore<F: HashFormat = Sha1>(gix_odb::memory::Proxy<NoBackend>, PhantomData<F>);

impl Default for ObjectStore<Sha1> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: HashFormat> std::fmt::Debug for ObjectStore<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStore")
            .field("kind", &F::KIND)
            .field("objects", &self.0.num_objects_in_memory())
            .finish()
    }
}

impl<F: HashFormat> ObjectStore<F> {
    /// Create an empty store that hashes new objects with format `F`.
    pub fn new() -> Self {
        Self(gix_odb::memory::Proxy::new(NoBackend, F::KIND), PhantomData)
    }

    /// Wrap a freshly hashed object id, mapping an impossible format mismatch
    /// (a backend that hashed with the wrong format) to an error rather than
    /// panicking.
    fn wrap(raw: ObjectId) -> Result<Oid<F>, SerializeError> {
        Oid::new(raw).map_err(|error| {
            SerializeError::Reflect(format!(
                "object store produced an id of the wrong hash format: {error}"
            ))
        })
    }

    /// Write `object` and return its format-checked id.
    ///
    /// This shadows the `gix` [`Write::write`] method for callers that hold a
    /// concrete [`ObjectStore`]; the trait method remains available (returning a
    /// bare [`ObjectId`]) through the [`Write`] bound.
    pub fn write(&self, object: &dyn WriteTo) -> Result<Oid<F>, SerializeError> {
        Self::wrap(self.0.write(object).map_err(SerializeError::Backend)?)
    }

    /// Write a raw buffer of `kind` and return its format-checked id.
    pub fn write_buf(&self, kind: Kind, from: &[u8]) -> Result<Oid<F>, SerializeError> {
        Self::wrap(
            self.0
                .write_buf(kind, from)
                .map_err(SerializeError::Backend)?,
        )
    }

    /// Write a stream of `kind` and return its format-checked id.
    pub fn write_stream(
        &self,
        kind: Kind,
        size: u64,
        from: &mut dyn Read,
    ) -> Result<Oid<F>, SerializeError> {
        Self::wrap(
            self.0
                .write_stream(kind, size, from)
                .map_err(SerializeError::Backend)?,
        )
    }

    /// Decode and return the object stored under `id`, if present.
    ///
    /// Corrupt and missing are distinguished: a failed fetch surfaces as
    /// [`DeserializeError::Backend`] and an undecodable object as
    /// [`DeserializeError::Decode`], so a corrupt-object diagnostic is
    /// possible through this API instead of every failure collapsing to
    /// `None`.
    pub fn get(&self, id: &Oid<F>) -> Result<Option<GitObject>, DeserializeError> {
        let mut buf = Vec::new();
        let Some(data) = self
            .0
            .try_find(id.as_oid(), &mut buf)
            .map_err(DeserializeError::Backend)?
        else {
            return Ok(None);
        };
        let decode = |source| DeserializeError::Decode {
            oid: id.as_oid().to_owned(),
            source,
        };
        data.decode()
            .map_err(decode)?
            .into_owned()
            .map_err(decode)
            .map(Some)
    }

    /// Return the entries of the tree stored under `id`, if it is a tree.
    ///
    /// Missing and wrong-kind are distinguished, as in [`get`](Self::get): an
    /// absent id is `Ok(None)`, while an object that exists but is not a tree
    /// is [`DeserializeError::NotATree`].
    pub fn get_tree(&self, id: &Oid<F>) -> Result<Option<Vec<TreeEntry>>, DeserializeError> {
        match self.get(id)? {
            Some(GitObject::Tree(tree)) => Ok(Some(tree.entries)),
            Some(_) => Err(DeserializeError::NotATree(id.as_oid().to_owned())),
            None => Ok(None),
        }
    }

    /// Return the raw bytes of the blob stored under `id`, if it is a blob.
    ///
    /// Missing and wrong-kind are distinguished, as in [`get`](Self::get): an
    /// absent id is `Ok(None)`, while an object that exists but is not a blob
    /// is [`DeserializeError::NotABlob`].
    pub fn get_blob(&self, id: &Oid<F>) -> Result<Option<Vec<u8>>, DeserializeError> {
        match self.get(id)? {
            Some(GitObject::Blob(blob)) => Ok(Some(blob.data)),
            Some(_) => Err(DeserializeError::NotABlob(id.as_oid().to_owned())),
            None => Ok(None),
        }
    }
}

impl<F: HashFormat> Find for ObjectStore<F> {
    fn try_find<'a>(
        &self,
        id: &gix_hash::oid,
        buffer: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
        self.0.try_find(id, buffer)
    }
}

impl<F: HashFormat> Write for ObjectStore<F> {
    fn write_stream(
        &self,
        kind: Kind,
        size: u64,
        from: &mut dyn Read,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.0.write_stream(kind, size, from)
    }

    fn write_buf_with_known_id(
        &self,
        kind: Kind,
        from: &[u8],
        id: ObjectId,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.0.write_buf_with_known_id(kind, from, id)
    }

    fn write_stream_with_known_id(
        &self,
        kind: Kind,
        size: u64,
        from: &mut dyn Read,
        id: ObjectId,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.0.write_stream_with_known_id(kind, size, from, id)
    }
}

impl<F: HashFormat> Tagged<F> for ObjectStore<F> {}

/// Inert backing database for [`ObjectStore`]'s in-memory [`Proxy`].
///
/// [`gix_odb::memory::Proxy`] is generic over an inner object database it falls
/// back to, but [`ObjectStore`] keeps everything in the proxy's in-memory map, so
/// the inner is never read from or written to. gitoxide ships no type that is
/// both [`Find`] and [`Write`] while doing nothing, so this supplies one.
///
/// [`Proxy`]: gix_odb::memory::Proxy
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

    fn write_buf_with_known_id(
        &self,
        _kind: Kind,
        _from: &[u8],
        _id: ObjectId,
    ) -> Result<ObjectId, gix_object::write::Error> {
        Err("NoBackend: writes are handled by the in-memory proxy".into())
    }

    fn write_stream_with_known_id(
        &self,
        _kind: Kind,
        _size: u64,
        _from: &mut dyn Read,
        _id: ObjectId,
    ) -> Result<ObjectId, gix_object::write::Error> {
        Err("NoBackend: writes are handled by the in-memory proxy".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Sha256;
    use crate::{EntryKind, EntryMode};
    use gix_object::Tree;

    fn hashes_and_reads<F: HashFormat>(blob_hex: &str, tree_hex: &str) {
        let store = ObjectStore::<F>::new();
        let blob = store.write_buf(Kind::Blob, b"hello").unwrap();
        assert_eq!(blob.kind(), F::KIND);
        assert_eq!(blob.to_string(), blob_hex);
        assert_eq!(
            store.get_blob(&blob).unwrap().as_deref(),
            Some(&b"hello"[..])
        );

        let tree = Tree {
            entries: ["a", "b"]
                .into_iter()
                .map(|name| TreeEntry {
                    mode: EntryMode::from(EntryKind::Blob),
                    filename: name.into(),
                    oid: blob.into_raw(),
                })
                .collect(),
        };
        let tree_id = store.write(&tree).unwrap();
        assert_eq!(tree_id.kind(), F::KIND);
        assert_eq!(tree_id.to_string(), tree_hex);
        assert_eq!(store.get_tree(&tree_id).unwrap(), Some(tree.entries));

        let mut buf = Vec::new();
        assert_eq!(
            store
                .try_find(tree_id.as_oid(), &mut buf)
                .unwrap()
                .unwrap()
                .object_hash,
            F::KIND
        );
    }

    #[test]
    fn hashes_and_reads_git_objects_in_both_formats() {
        hashes_and_reads::<Sha1>(
            "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0",
            "bc787e446f83ed260a7e22c65583e1bf1423813e",
        );
        hashes_and_reads::<Sha256>(
            "8aec4e4876f854f688d0ebfc8f37598f38e5fd6903cccc850ca36591175aeb60",
            "1203e33ca4c968086cd181967aba60b4e14a5fb0c355c152d15db69f7ac6f639",
        );
    }

    #[test]
    fn default_store_uses_sha1() {
        let store = ObjectStore::default();
        let id = store.write_buf(Kind::Blob, b"hello").unwrap();
        assert_eq!(id.kind(), gix_hash::Kind::Sha1);
    }

    #[test]
    fn foreign_kind_objects_are_reachable_only_through_raw_find() {
        let source = ObjectStore::<Sha256>::new();
        let blob = source.write_buf(Kind::Blob, b"hello").unwrap();
        let tree = Tree {
            entries: vec![TreeEntry {
                mode: EntryMode::from(EntryKind::Blob),
                filename: "hello".into(),
                oid: blob.into_raw(),
            }],
        };
        let id = source.write(&tree).unwrap();
        let mut buf = Vec::new();
        let data = source.try_find(id.as_oid(), &mut buf).unwrap().unwrap();
        let raw = id.into_raw();

        // Known-ID writes intentionally retain gix's ability to store an object
        // under an ID of a different kind from the store's normal write format.
        // The raw `Find` interface still returns it, but it can never be named
        // as an `Oid<Sha1>`, so no format-checked accessor can be misused on it.
        let target = ObjectStore::default();
        target
            .write_buf_with_known_id(Kind::Tree, data.data, raw)
            .unwrap();
        let mut buf = Vec::new();
        assert!(target.try_find(raw.as_ref(), &mut buf).unwrap().is_some());
        assert!(Oid::<Sha1>::new(raw).is_err());
    }

    #[test]
    fn malformed_tree_is_a_decode_error_for_either_hash_kind() {
        fn malformed<F: HashFormat>() {
            let store = ObjectStore::<F>::new();
            let id = store.write_buf(Kind::Tree, b"100644 bad\0short").unwrap();
            assert!(matches!(
                store.get(&id),
                Err(DeserializeError::Decode { .. })
            ));
        }
        malformed::<Sha1>();
        malformed::<Sha256>();
    }
}
