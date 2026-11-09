//! The `.schema` document envelope.
//!
//! A *document* is a Git tree whose entries are the entries of a value's own
//! bare encoding, plus one reserved entry, `.schema`, naming the unbound schema
//! the value conforms to:
//!
//! ```text
//! { <value's own entries>..., .schema -> <unbound schema> }
//! ```
//!
//! There is no `value/` subtree: a document's schema root resolves to a struct,
//! so the value's fields sit directly at the root beside `.schema` and
//! `git ls-tree` shows them there. `.value` is reserved and never written.
//!
//! The bare-value codec ([`serialize`](crate::serialize) and
//! [`deserialize`](crate::deserialize)) knows nothing of this entry, so a reader
//! of a document works through [`UnboundValue`], a view that hides `.schema`;
//! [`read_value`] wires that view to a schema-directed read. Classification is
//! whole-name — see `docs/format.adoc` — so a user field literally named
//! `.schema` is escaped to `%.schema` and can never be mistaken for the
//! envelope entry.

use gix_object::tree::Entry;
use gix_object::{Data, Find, Kind, Tree, TreeRef, WriteTo};

use crate::ObjectId;
use crate::error::{DocumentError, Subtree};
use crate::hash::{HashFormat, Tagged};
use crate::schema::{Node, Schema};

/// The reserved root entry naming a document's unbound schema.
pub const SCHEMA_ENTRY: &str = Subtree::Schema.as_str();

/// Refuse a schema that cannot serve as a document root.
///
/// Only a struct root has named entries that can sit beside `.schema`; any
/// other resolved root describes a bare value, not a document. A root that
/// does not resolve at all — a `ref` naming a definition absent from `defs`,
/// or a ref cycle — can likewise never form a document, and is reported as
/// [`DocumentError::SchemaRootUnresolved`] rather than published and failing
/// every later read.
pub fn ensure_struct_root(doc: &Schema) -> Result<(), DocumentError> {
    match doc.resolved_root() {
        Ok(Node::Struct(_)) => Ok(()),
        Ok(node) => Err(DocumentError::DocumentRootNotStruct {
            kind: doc.kind().to_owned(),
            node: node.keyword(),
        }),
        Err(source) => Err(DocumentError::SchemaRootUnresolved {
            kind: doc.kind().to_owned(),
            source,
        }),
    }
}

/// Find the object kind of `oid`, or `None` when it is absent.
fn object_kind<S: Find + ?Sized>(
    objects: &S,
    oid: ObjectId,
) -> Result<Option<Kind>, DocumentError> {
    let mut buf = Vec::new();
    Ok(objects
        .try_find(&oid, &mut buf)
        .map_err(DocumentError::Backend)?
        .map(|data| data.kind))
}

/// Split a document root into the root itself and its embedded schema tree.
///
/// This is the *validating parser* the read path is built on: it accepts only a
/// well-formed envelope and rejects everything else. A tree with no `.schema`
/// entry is reported as [`DocumentError::NotSubtreeBound`] — the absence of an
/// envelope — rather than as a value. A malformed envelope (a `.schema` that is
/// not a tree, or a root with no entries beside it) is rejected the same way;
/// requiring the whole shape, rather than looking up `.schema` in isolation, is
/// what makes a bare value tree fail as one diagnosable error instead of being
/// half-matched and misreported.
///
/// The root must carry a `.schema` entry holding a tree, and at least one other
/// entry for the value; every other entry is the value's own.
///
/// The returned root is the *document* root, not a bare value tree: reading its
/// value goes through [`UnboundValue`].
pub fn split<S: Find + ?Sized>(
    root: ObjectId,
    objects: &S,
) -> Result<(ObjectId, ObjectId), DocumentError> {
    let mut buf = Vec::new();
    let data = objects
        .try_find(&root, &mut buf)
        .map_err(DocumentError::Backend)?
        .ok_or(DocumentError::MissingObject { oid: root })?;
    if data.kind != Kind::Tree {
        return Err(DocumentError::NotATree { oid: root });
    }
    let tree = TreeRef::from_bytes(data.data, data.object_hash)
        .map_err(|source| DocumentError::Decode { oid: root, source })?;
    let not_bound = || DocumentError::NotSubtreeBound {
        root,
        found: tree
            .entries
            .iter()
            .map(|entry| String::from_utf8_lossy(entry.filename).into_owned())
            .collect::<Vec<_>>()
            .join(", "),
    };
    // A struct emits at least one entry (the `none` marker for an empty one), so
    // `.schema` is never a root's only entry.
    if tree.entries.len() < 2 {
        return Err(not_bound());
    }
    let schema = tree
        .entries
        .iter()
        .find(|entry| entry.filename == SCHEMA_ENTRY.as_bytes())
        .ok_or_else(not_bound)?;
    if !schema.mode.is_tree() {
        return Err(not_bound());
    }
    let schema = schema.oid.to_owned();
    match object_kind(objects, schema)? {
        Some(Kind::Tree) => {}
        Some(_) => return Err(DocumentError::NotATree { oid: schema }),
        None => {
            return Err(DocumentError::SubtreeMissing {
                subtree: Subtree::Schema,
                oid: schema,
                root,
            });
        }
    }
    Ok((root, schema))
}

/// Decode the value a bound document root encloses, under `doc`.
///
/// The schema root must resolve to a struct; the root's own entries, with the
/// reserved `.schema` entry hidden, are then handed to `read` as the value's
/// tree. A caller supplies its own reading step — a typed decode, a
/// schema-directed decode, or a store's encoding — and unifies the envelope's
/// [`DocumentError`] with whatever that step reports through `E`.
pub fn read_value<T, E, S>(
    root: ObjectId,
    doc: &Schema,
    objects: &S,
    read: impl FnOnce(&ObjectId, &Schema, &UnboundValue<'_, S>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<DocumentError>,
    S: Find + ?Sized,
{
    ensure_struct_root(doc)?;
    let value = UnboundValue::new(root, objects)?;
    read(&root, doc, &value)
}

/// A document root presented to a bare-value codec as the value it encloses.
///
/// The value's own entries sit in the document root beside `.schema`, so a
/// reader cannot be pointed at that root directly. This view answers for the
/// root's own id with a tree holding exactly the value's entries — `.schema`
/// removed — while delegating every other object to the underlying store. It
/// writes nothing and leaves every other object untouched.
///
/// A subtree can never share the root's id (Git's object graph is acyclic and
/// content-addressed), so intercepting that one id cannot shadow a value's own
/// subtree.
pub struct UnboundValue<'a, S: ?Sized> {
    inner: &'a S,
    root: ObjectId,
    bytes: Vec<u8>,
}

impl<'a, S: Find + ?Sized> UnboundValue<'a, S> {
    /// View `root`'s entries with `.schema` removed, as one synthesized tree.
    pub fn new(root: ObjectId, objects: &'a S) -> Result<Self, DocumentError> {
        let mut buf = Vec::new();
        let data = objects
            .try_find(&root, &mut buf)
            .map_err(DocumentError::Backend)?
            .ok_or(DocumentError::MissingObject { oid: root })?;
        if data.kind != Kind::Tree {
            return Err(DocumentError::NotATree { oid: root });
        }
        let tree = TreeRef::from_bytes(data.data, data.object_hash)
            .map_err(|source| DocumentError::Decode { oid: root, source })?;
        let entries: Vec<Entry> = tree
            .entries
            .iter()
            .filter(|entry| entry.filename != SCHEMA_ENTRY.as_bytes())
            .map(|entry| Entry {
                mode: entry.mode,
                filename: entry.filename.into(),
                oid: entry.oid.to_owned(),
            })
            .collect();
        // Removing an entry preserves Git's entry order, so the synthesized
        // tree is canonical exactly when the document root was.
        let mut bytes = Vec::new();
        Tree { entries }
            .write_to(&mut bytes)
            .expect("writing a decoded tree to memory cannot fail");
        Ok(Self {
            inner: objects,
            root,
            bytes,
        })
    }
}

/// The view stands in for its inner store, so a schema-directed reader that
/// requires a hash-format tag accepts it exactly when it accepts the store.
impl<F: HashFormat, S: Tagged<F> + ?Sized> Tagged<F> for UnboundValue<'_, S> {}

impl<S: Find + ?Sized> Find for UnboundValue<'_, S> {
    fn try_find<'b>(
        &self,
        id: &gix_hash::oid,
        buffer: &'b mut Vec<u8>,
    ) -> Result<Option<Data<'b>>, gix_object::find::Error> {
        if id == self.root.as_ref() {
            buffer.clear();
            buffer.extend_from_slice(&self.bytes);
            return Ok(Some(Data {
                data: buffer.as_slice(),
                kind: Kind::Tree,
                object_hash: self.root.kind(),
            }));
        }
        self.inner.try_find(id, buffer)
    }
}
