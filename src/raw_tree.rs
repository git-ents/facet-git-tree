//! [`RawTree`], the raw-passthrough tree field.

use facet::Facet;

use crate::ObjectId;

/// A Git tree already written into the backing store, embedded by object id
/// rather than walked field-by-field, so a `Facet`-derived struct can embed
/// an arbitrarily-shaped subtree next to ordinarily-encoded fields. The
/// serializer and deserializer intercept this type by shape identity and
/// pass the wrapped object id straight through as a tree entry. The referenced
/// tree must already exist in the store being written to and use the same hash
/// format as that store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct RawTree {
    #[facet(opaque)]
    hash: ObjectId,
}

impl RawTree {
    /// Wrap a tree's SHA-1 or SHA-256 object id for embedding as a passthrough field.
    pub fn new(oid: ObjectId) -> Self {
        Self { hash: oid }
    }

    /// The wrapped tree's object id.
    pub fn oid(&self) -> ObjectId {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectStore, deserialize, serialize_into};
    use gix_hash::Kind as HashKind;
    use gix_object::Write;

    #[derive(Debug, Facet, PartialEq)]
    struct WithRawTree {
        content: RawTree,
    }

    #[test]
    fn preserves_ids_and_facet_identity_in_both_hash_formats() {
        for hash_kind in [HashKind::Sha1, HashKind::Sha256] {
            let store = ObjectStore::with_hash_kind(hash_kind);
            // Passthrough trees may be empty even though ordinary encoded trees may not.
            let id = store.write(&gix_object::Tree { entries: vec![] }).unwrap();
            let value = WithRawTree {
                content: RawTree::new(id),
            };
            assert_eq!(value.content.oid(), id);

            let root = serialize_into(&value, &store).unwrap();
            let entries = store.get_tree(&root).unwrap().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].oid, id);
            assert_eq!(deserialize::<WithRawTree>(&root, &store).unwrap(), value);
        }
    }
}
