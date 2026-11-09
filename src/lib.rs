//! Serialize [`facet::Facet`] values to, and deserialize them from, Git trees.
//!
//! A value is encoded as a graph of Git objects — scalars and strings as blobs,
//! structs, enums, and collections as trees — addressed exactly as Git would
//! compute them, in either the SHA-1 or the SHA-256 object format. The bundled
//! [`ObjectStore`] is an in-memory backend, but the entry points are generic
//! over `gix`'s [`Find`](gix_object::Find) and [`Write`](gix_object::Write)
//! traits, so a real `gix` repository or object database works just as well.
//!
//! The on-disk layout is the frozen format in `docs/format.adoc`; there is no
//! backward compatibility with earlier layouts.
//!
//! # Strictness
//!
//! Reads are strict: a struct emits every field, so a missing field and an
//! unknown entry are both errors ([`DeserializeError::MissingField`],
//! [`DeserializeError::UnexpectedEntry`]); tree entries must be correctly
//! ordered and sized; and leaf text must be canonical.
//!
//! Beyond bare values the crate owns the `.schema` *document envelope*
//! ([`document`]): a document is a value's own entries beside one reserved
//! `.schema` entry naming the unbound schema it conforms to. A document's
//! schema root must be a struct, so its fields sit at the document root beside
//! `.schema`. Refs, commits, placement, and policy are the store layer's
//! ([`git-store`](https://github.com/git-ents/git-store)), which builds on this
//! crate.
#![forbid(unsafe_code)]

pub mod attr;
mod cbor;
mod classify;
mod de;
pub mod doctor;
pub mod document;
mod error;
mod float;
mod hash;
mod limits;
mod marker;
mod name;
pub mod normal_form;
mod ordinal;
mod raw_blob;
mod raw_tree;
pub mod schema;
mod ser;
mod store;

pub use gix_hash::ObjectId;
pub use gix_object::Object as GitObject;
pub use gix_object::tree::{Entry as TreeEntry, EntryKind, EntryMode};

pub use de::{check_key, deserialize};
pub use doctor::{
    DoctorError, DoctorReport, EncodeStage, S0_SHA1, S0_SHA256, check, check_against,
};
pub use error::{
    DeserializeError, DocumentError, KeyError, NormalFormError, SchemaError, SchemaReadError,
    SchemaWriteError, SerializeError, Subtree, UniverseError,
};
pub use hash::{Backend, HashFormat, HashFormatMismatch, Oid, Sha1, Sha256, Tagged};
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
pub use schema::{CheckedSchema, Node, RefError, Schema, VariantKind, schema_of};
pub use ser::{serialize, serialize_into};
pub use store::ObjectStore;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
