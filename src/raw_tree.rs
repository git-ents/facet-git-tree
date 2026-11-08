//! [`RawTree`], the raw-passthrough tree field.

use facet::Facet;

use crate::hash::{HashFormat, Oid, Sha1};

/// A Git tree already written into the backing store, embedded by object id
/// rather than walked field-by-field, so a `Facet`-derived struct can embed
/// an arbitrarily-shaped subtree next to ordinarily-encoded fields. The
/// serializer and deserializer intercept this type by shape identity and
/// pass the wrapped object id straight through as a tree entry.
///
/// The wrapped [`Oid<F>`] fixes the tree's hash format in the type, so a tree
/// of one format cannot be embedded in a document serialized into a store of
/// the other. The referenced tree must still already exist in the store being
/// written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[facet(opaque)]
pub struct RawTree<F: HashFormat = Sha1> {
    hash: Oid<F>,
}

impl<F: HashFormat> RawTree<F> {
    /// Wrap a tree's object id (already checked to use format `F`) for
    /// embedding as a passthrough field.
    pub fn new(oid: Oid<F>) -> Self {
        Self { hash: oid }
    }

    /// The wrapped tree's object id.
    pub fn oid(&self) -> Oid<F> {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Sha256;
    use crate::{ObjectStore, deserialize, serialize_into};

    #[derive(Debug, Facet, PartialEq)]
    struct WithRawTree {
        content: RawTree,
    }

    #[derive(Debug, Facet, PartialEq)]
    struct WithRawTree256 {
        content: RawTree<Sha256>,
    }

    #[test]
    fn sha1_preserves_ids_and_facet_identity() {
        let store = ObjectStore::<Sha1>::new();
        // Passthrough trees may be empty even though ordinary encoded trees may not.
        let id = store.write(&gix_object::Tree { entries: vec![] }).unwrap();
        let value = WithRawTree {
            content: RawTree::new(id),
        };
        assert_eq!(value.content.oid(), id);

        let root = serialize_into(&value, &store).unwrap();
        let entries = store.get_tree(&root).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].oid, id.into_raw());
        assert_eq!(
            deserialize::<WithRawTree, _, _>(&root, &store).unwrap(),
            value
        );
    }

    #[test]
    fn sha256_preserves_ids_and_facet_identity() {
        let store = ObjectStore::<Sha256>::new();
        let id = store.write(&gix_object::Tree { entries: vec![] }).unwrap();
        let value = WithRawTree256 {
            content: RawTree::new(id),
        };
        assert_eq!(value.content.oid(), id);

        let root = serialize_into(&value, &store).unwrap();
        let entries = store.get_tree(&root).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].oid, id.into_raw());
        assert_eq!(
            deserialize::<WithRawTree256, _, _>(&root, &store).unwrap(),
            value
        );
    }
}
