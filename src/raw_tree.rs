//! [`RawTree`], the raw-passthrough tree field.

use facet::Facet;

use crate::ObjectId;

/// A Git tree already written into the backing store, embedded by object id
/// rather than walked field-by-field, so a `Facet`-derived struct can embed
/// an arbitrarily-shaped subtree next to ordinarily-encoded fields. The
/// serializer and deserializer intercept this type by shape identity and
/// pass the wrapped object id straight through as a tree entry; the
/// referenced tree must already exist in the store being written to. Sha-1
/// only, like the rest of this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct RawTree {
    hash: [u8; 20],
}

impl RawTree {
    /// Wrap a tree's object id for embedding as a passthrough field.
    pub fn new(oid: ObjectId) -> Self {
        let mut hash = [0u8; 20];
        hash.copy_from_slice(oid.as_slice());
        Self { hash }
    }

    /// The wrapped tree's object id.
    pub fn oid(&self) -> ObjectId {
        ObjectId::from_bytes_or_panic(&self.hash)
    }
}
