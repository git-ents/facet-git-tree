//! [`RawBlob`], the raw-passthrough blob field.

use facet::Facet;

use crate::ObjectId;

/// A Git blob already written into the backing store, embedded by object id
/// rather than walked field-by-field: the serializer and deserializer
/// intercept this type by shape identity, pass the wrapped object id
/// straight through as a blob entry. The referenced object must already exist
/// in the store being written to and use the same hash format as that store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct RawBlob {
    #[facet(opaque)]
    hash: ObjectId,
}

impl RawBlob {
    /// Wrap a blob's SHA-1 or SHA-256 object id for embedding as a passthrough field.
    pub fn new(oid: ObjectId) -> Self {
        Self { hash: oid }
    }

    /// The wrapped blob's object id.
    pub fn oid(&self) -> ObjectId {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectStore, deserialize, serialize_into};
    use gix_hash::Kind as HashKind;
    use gix_object::{Kind, Write};

    #[derive(Debug, Facet, PartialEq)]
    struct WithRawBlob {
        content: RawBlob,
    }

    #[test]
    fn preserves_ids_and_facet_identity_in_both_hash_formats() {
        for hash_kind in [HashKind::Sha1, HashKind::Sha256] {
            let store = ObjectStore::with_hash_kind(hash_kind);
            let id = store.write_buf(Kind::Blob, b"raw\0bytes").unwrap();
            let value = WithRawBlob {
                content: RawBlob::new(id),
            };
            assert_eq!(value.content.oid(), id);

            let root = serialize_into(&value, &store).unwrap();
            let entries = store.get_tree(&root).unwrap().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].oid, id);
            assert_eq!(deserialize::<WithRawBlob>(&root, &store).unwrap(), value);
        }
    }
}
