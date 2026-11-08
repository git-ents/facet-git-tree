//! [`RawBlob`], the raw-passthrough blob field.

use facet::Facet;

use crate::hash::{HashFormat, Oid, Sha1};

/// A Git blob already written into the backing store, embedded by object id
/// rather than walked field-by-field: the serializer and deserializer
/// intercept this type by shape identity and pass the wrapped object id
/// straight through as a blob entry.
///
/// The wrapped [`Oid<F>`] fixes the object's hash format in the type, so a
/// blob of one format cannot be embedded in a document serialized into a store
/// of the other. The referenced object must still already exist in the store
/// being written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[facet(opaque)]
pub struct RawBlob<F: HashFormat = Sha1> {
    hash: Oid<F>,
}

impl<F: HashFormat> RawBlob<F> {
    /// Wrap a blob's object id (already checked to use format `F`) for
    /// embedding as a passthrough field.
    pub fn new(oid: Oid<F>) -> Self {
        Self { hash: oid }
    }

    /// The wrapped blob's object id.
    pub fn oid(&self) -> Oid<F> {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Sha256;
    use crate::{ObjectStore, deserialize, serialize_into};
    use gix_object::Kind;

    #[derive(Debug, Facet, PartialEq)]
    struct WithRawBlob {
        content: RawBlob,
    }

    #[derive(Debug, Facet, PartialEq)]
    struct WithRawBlob256 {
        content: RawBlob<Sha256>,
    }

    #[test]
    fn sha1_preserves_ids_and_facet_identity() {
        let store = ObjectStore::<Sha1>::new();
        let id = store.write_buf(Kind::Blob, b"raw\0bytes").unwrap();
        let value = WithRawBlob {
            content: RawBlob::new(id),
        };
        assert_eq!(value.content.oid(), id);

        let root = serialize_into(&value, &store).unwrap();
        let entries = store.get_tree(&root).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].oid, id.into_raw());
        assert_eq!(
            deserialize::<WithRawBlob, _, _>(&root, &store).unwrap(),
            value
        );
    }

    #[test]
    fn sha256_preserves_ids_and_facet_identity() {
        let store = ObjectStore::<Sha256>::new();
        let id = store.write_buf(Kind::Blob, b"raw\0bytes").unwrap();
        let value = WithRawBlob256 {
            content: RawBlob::new(id),
        };
        assert_eq!(value.content.oid(), id);

        let root = serialize_into(&value, &store).unwrap();
        let entries = store.get_tree(&root).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].oid, id.into_raw());
        assert_eq!(
            deserialize::<WithRawBlob256, _, _>(&root, &store).unwrap(),
            value
        );
    }
}
