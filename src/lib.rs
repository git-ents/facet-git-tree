//! Serialize [`facet::Facet`] values to, and deserialize them from, Git trees.
//!
//! A value is encoded as a graph of Git objects — scalars and strings as blobs,
//! structs, enums, and collections as trees — addressed by SHA-1 exactly as Git
//! would compute them. The bundled [`ObjectStore`] is an in-memory backend, but
//! the entry points are generic over `gix`'s `Find` and `Write` traits, so a
//! real `gix` repository or object database works just as well.
//!
//! The normative encoding rules live in `docs/specification.adoc`.
//!
//! # Design notes
//!
//! Three properties of the mapping are worth stating explicitly, because
//! they are the subtle wins (and the one deliberate cost) of encoding into
//! Git's object model rather than inventing a container format over it:
//!
//! * **Unit enum variants are bare blobs** holding the variant name — their
//!   entire information content — rather than trees wrapping it. Git's
//!   `ls-tree -r` and `diff` are blob-oriented, so a variant flip shows up
//!   as an ordinary one-line content change instead of vanishing as a
//!   tree-entry rename with no content on either side.
//!
//! * **Emptiness is a blob, not an absence**: `None`, dynamic `Null`, and
//!   empty collections are written as a one-entry presence-marker tree
//!   (see `marker`) instead of a literal empty tree, which contributes
//!   nothing to `ls-tree -r` or `diff` — so a field going empty is
//!   visible in exactly the way a field never existing is not.
//!
//! * **Every `Some` costs a tree level** (`field/some/blob`): the `some`
//!   wrapper is what keeps a defaulted field's absence (no entry at all)
//!   distinguishable from an explicit `None` (the marker). That is one
//!   extra hop per optional value in every tree listing — a deliberate
//!   trade of a little depth for the defaulted-field ambiguity it removes.
//!
//! One documented asymmetry: a dynamic `DateTime` is written as RFC 3339
//! text, but the schema language has no `Node::DateTime`, so a
//! schema-directed read recovers it as a `String` — the schema cannot
//! describe a value the codec writes. Lossy by design, and stated here so
//! the asymmetry is visible at the crate boundary, not only at the error
//! variant.
#![forbid(unsafe_code)]

pub mod attr;
mod classify;
mod de;
mod error;
mod limits;
mod marker;
pub mod migration;
pub mod normal_form;
mod raw_blob;
mod raw_tree;
pub mod schema;
mod ser;
mod store;

pub use gix_hash::ObjectId;
pub use gix_object::Object as GitObject;
pub use gix_object::tree::{Entry as TreeEntry, EntryKind, EntryMode};

pub use de::{DecodeMode, check_key, deserialize, deserialize_into, deserialize_legacy_leaves};
pub use error::{
    DeserializeError, KeyError, MigrationError, MigrationPinError, NormalFormError, SchemaError,
    SchemaPinError, SchemaReadError, SchemaWriteError, SerializeError, UniverseError,
};
#[cfg(feature = "value")]
pub use migration::apply::{Edge, apply, apply_chain};
pub use migration::derive::{Derivation, Divergence, Incomplete, Side};
pub use migration::pin::MigrationSchema;
pub use migration::{Change, Constant, Hints, Migration, Op, Target};
pub use normal_form::{
    IDENTITY_DEF_PREFIX, Key, NormalForm, check_identity_subtrees, check_universe,
    check_universe_at, identity_subtrees,
};
pub use raw_blob::RawBlob;
pub use raw_tree::RawTree;
pub use schema::pin::{EMPTY_TREE, SchemaSchema};
#[cfg(feature = "value")]
pub use schema::read::{
    deserialize_value_with_schema, deserialize_value_with_schema_legacy_leaves,
    validate_with_schema,
};
#[cfg(feature = "value")]
pub use schema::write::serialize_value_with_schema;
pub use schema::{Node, Schema, StructField, VariantKind, schema_and_hints_of, schema_of};
pub use ser::{serialize, serialize_into, serialize_peek, serialize_peek_into};
pub use store::ObjectStore;
