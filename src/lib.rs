//! Serialize [`facet::Facet`] values to, and deserialize them from, Git trees.
//!
//! A value is encoded as a graph of Git objects — scalars and strings as blobs,
//! structs, enums, and collections as trees — addressed by SHA-1 exactly as Git
//! would compute them. The bundled [`ObjectStore`] is an in-memory backend, but
//! the entry points are generic over `gix`'s `Find` and `Write` traits, so a
//! real `gix` repository or object database works just as well.
//!
//! # Field matching
//!
//! Typed reads are strict: a missing field errors unless it has a `facet`
//! default or is an `Option`; an unmatched tree entry errors with
//! [`DeserializeError::UnexpectedEntry`]; a duplicated composite map key
//! errors on both write ([`SerializeError::DuplicateKey`]) and read.
#![forbid(unsafe_code)]

pub mod attr;
mod classify;
mod de;
mod error;
mod limits;
mod marker;
pub mod normal_form;
mod raw_blob;
mod raw_tree;
pub mod schema;
mod ser;
mod store;

pub use gix_hash::ObjectId;
pub use gix_object::Object as GitObject;
pub use gix_object::tree::{Entry as TreeEntry, EntryKind, EntryMode};

pub use de::{check_key, deserialize, deserialize_into};
pub use error::{
    DeserializeError, KeyError, NormalFormError, SchemaError, SchemaReadError, SchemaWriteError,
    SerializeError, UniverseError,
};
pub use normal_form::{
    IDENTITY_DEF_PREFIX, Key, NormalForm, check_identity_subtrees, check_universe,
    identity_subtrees,
};
pub use raw_blob::RawBlob;
pub use raw_tree::RawTree;
#[cfg(feature = "value")]
pub use schema::read::{deserialize_value_with_schema, validate_with_schema};
#[cfg(feature = "value")]
pub use schema::write::serialize_value_with_schema;
pub use schema::{Node, Schema, StructField, VariantKind, schema_of};
pub use ser::{serialize, serialize_into};
pub use store::ObjectStore;
