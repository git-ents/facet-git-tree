//! [`RawBlob`], the raw-passthrough blob field.

use facet::Facet;

use crate::ObjectId;

/// A Git blob already written into the backing store, embedded by object id
/// rather than walked field-by-field: the serializer and deserializer
/// intercept this type by shape identity, pass the wrapped object id
/// straight through as a blob entry, and the referenced object must already
/// exist in the store being written to. Sha-1 only, like the rest of this
/// crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct RawBlob {
    hash: [u8; 20],
}

impl RawBlob {
    /// Wrap a blob's object id for embedding as a passthrough field.
    pub fn new(oid: ObjectId) -> Self {
        let mut hash = [0u8; 20];
        hash.copy_from_slice(oid.as_slice());
        Self { hash }
    }

    /// The wrapped blob's object id.
    pub fn oid(&self) -> ObjectId {
        ObjectId::from_bytes_or_panic(&self.hash)
    }
}
