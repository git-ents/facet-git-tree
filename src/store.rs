//! The bundled in-memory [`ObjectStore`] backend.

use std::io::Read;

use gix_hash::Kind as HashKind;
use gix_object::{Data, Find, Kind, Write};

use crate::error::DeserializeError;
use crate::{GitObject, ObjectId, TreeEntry};

/// A content-addressed store of Git objects produced by
/// [`serialize`](crate::serialize).
///
/// This is a thin wrapper around [`gix_odb::memory::Proxy`], gitoxide's own
/// in-memory object database, so the `Find`/`Write` buffer handling lives in
/// `gix` rather than being reimplemented here. The accessors return owned values
/// because the proxy is `RefCell`-backed (required since [`Write::write_stream`]
/// takes `&self`).
///
/// This type is only a convenience default for callers that lack a backend of
/// their own. The actual contract is the generic `gix` [`Find`]/[`Write`] bounds
/// on [`serialize_into`](crate::serialize_into) and
/// [`deserialize`](crate::deserialize), which a real `gix` repository or
/// odb satisfies just as well. Like `gix`'s in-memory store it is `!Sync`;
/// cross-thread sharing is the job of the on-disk backends, not of this type.
pub struct ObjectStore(gix_odb::memory::Proxy<NoBackend>);

impl Default for ObjectStore {
    fn default() -> Self {
        Self::with_hash_kind(HashKind::Sha1)
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
    /// Create an empty store that hashes new objects with `hash_kind`.
    ///
    /// [`Default`] selects SHA-1; pass [`HashKind::Sha256`] for SHA-256.
    /// As with `gix`'s proxy, writes with a known ID trust the caller's ID.
    pub fn with_hash_kind(hash_kind: HashKind) -> Self {
        Self(gix_odb::memory::Proxy::new(NoBackend, hash_kind))
    }

    /// Decode and return the object stored under `id`, if present.
    ///
    /// Corrupt and missing are distinguished: a failed fetch surfaces as
    /// [`DeserializeError::Backend`] and an undecodable object as
    /// [`DeserializeError::Decode`], so a corrupt-object diagnostic is
    /// possible through this API instead of every failure collapsing to
    /// `None`.
    pub fn get(&self, id: &ObjectId) -> Result<Option<GitObject>, DeserializeError> {
        let mut buf = Vec::new();
        let Some(data) = self
            .0
            .try_find(id, &mut buf)
            .map_err(DeserializeError::Backend)?
        else {
            return Ok(None);
        };
        let decode = |source| DeserializeError::Decode { oid: *id, source };
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
    pub fn get_tree(&self, id: &ObjectId) -> Result<Option<Vec<TreeEntry>>, DeserializeError> {
        match self.get(id)? {
            Some(GitObject::Tree(tree)) => Ok(Some(tree.entries)),
            Some(_) => Err(DeserializeError::NotATree(*id)),
            None => Ok(None),
        }
    }

    /// Return the raw bytes of the blob stored under `id`, if it is a blob.
    ///
    /// Missing and wrong-kind are distinguished, as in [`get`](Self::get): an
    /// absent id is `Ok(None)`, while an object that exists but is not a blob
    /// is [`DeserializeError::NotABlob`].
    pub fn get_blob(&self, id: &ObjectId) -> Result<Option<Vec<u8>>, DeserializeError> {
        match self.get(id)? {
            Some(GitObject::Blob(blob)) => Ok(Some(blob.data)),
            Some(_) => Err(DeserializeError::NotABlob(*id)),
            None => Ok(None),
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
    use crate::{EntryKind, EntryMode};
    use gix_object::Tree;

    #[test]
    fn hashes_and_reads_git_objects_in_both_formats() {
        let cases = [
            (
                HashKind::Sha1,
                "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0",
                "bc787e446f83ed260a7e22c65583e1bf1423813e",
            ),
            (
                HashKind::Sha256,
                "8aec4e4876f854f688d0ebfc8f37598f38e5fd6903cccc850ca36591175aeb60",
                "1203e33ca4c968086cd181967aba60b4e14a5fb0c355c152d15db69f7ac6f639",
            ),
        ];
        for (hash_kind, blob_hex, tree_hex) in cases {
            let store = ObjectStore::with_hash_kind(hash_kind);
            let blob = store.write_buf(Kind::Blob, b"hello").unwrap();
            assert_eq!(blob.kind(), hash_kind);
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
                        oid: blob,
                    })
                    .collect(),
            };
            let tree_id = store.write(&tree).unwrap();
            assert_eq!(tree_id.kind(), hash_kind);
            assert_eq!(tree_id.to_string(), tree_hex);
            assert_eq!(store.get_tree(&tree_id).unwrap(), Some(tree.entries));

            let mut buf = Vec::new();
            assert_eq!(
                store
                    .try_find(&tree_id, &mut buf)
                    .unwrap()
                    .unwrap()
                    .object_hash,
                hash_kind
            );
        }
    }

    #[test]
    fn default_store_uses_sha1() {
        let store = ObjectStore::default();
        let id = store.write_buf(Kind::Blob, b"hello").unwrap();
        assert_eq!(id.kind(), HashKind::Sha1);
    }

    #[test]
    fn get_decodes_using_the_fetched_objects_hash_kind() {
        let source = ObjectStore::with_hash_kind(HashKind::Sha256);
        let blob = source.write_buf(Kind::Blob, b"hello").unwrap();
        let tree = Tree {
            entries: vec![TreeEntry {
                mode: EntryMode::from(EntryKind::Blob),
                filename: "hello".into(),
                oid: blob,
            }],
        };
        let id = source.write(&tree).unwrap();
        let mut buf = Vec::new();
        let data = source.try_find(&id, &mut buf).unwrap().unwrap();

        // Known-ID writes intentionally retain gix's ability to store an object
        // under an ID of a different kind from the store's normal write format.
        let target = ObjectStore::default();
        target
            .write_buf_with_known_id(Kind::Tree, data.data, id)
            .unwrap();
        assert_eq!(target.get_tree(&id).unwrap(), Some(tree.entries));
    }

    #[test]
    fn malformed_tree_is_a_decode_error_for_either_hash_kind() {
        for hash_kind in [HashKind::Sha1, HashKind::Sha256] {
            let store = ObjectStore::with_hash_kind(hash_kind);
            let id = store.write_buf(Kind::Tree, b"100644 bad\0short").unwrap();
            assert!(matches!(
                store.get(&id),
                Err(DeserializeError::Decode { .. })
            ));
        }
    }
}
