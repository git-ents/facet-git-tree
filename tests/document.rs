//! Integration tests for the `.schema` document envelope
//! (`facet_git_tree::document`).
//!
//! Covers the `== Envelope` rules of `docs/format.adoc`: a document is a
//! value's own entries beside one reserved `.schema` entry found by name, the
//! value's schema root resolves to a struct, and a bare value tree is not a
//! document.

use facet::Facet;
use facet_git_tree::document::{self, UnboundValue};
use facet_git_tree::{
    DocumentError, EntryKind, EntryMode, ObjectStore, Oid, RefError, Schema, Sha1, Subtree,
    TreeEntry, deserialize, schema_of, serialize_into,
};
use gix_object::{Find, Kind, Tree, TreeRef};

type Store = ObjectStore<Sha1>;

#[derive(Debug, Facet, PartialEq)]
struct Book {
    title: String,
    pages: u32,
}

/// Write a tree from `(name, kind, target)` entries in Git's entry order.
fn write_tree(store: &Store, entries: Vec<(&str, EntryKind, Oid<Sha1>)>) -> Oid<Sha1> {
    let mut entries: Vec<TreeEntry> = entries
        .into_iter()
        .map(|(name, kind, oid)| TreeEntry {
            mode: EntryMode::from(kind),
            filename: name.into(),
            oid: oid.to_raw(),
        })
        .collect();
    entries.sort();
    store.write(&Tree { entries }).expect("write tree")
}

/// The `{ none -> <empty blob> }` marker an empty collection encodes as.
fn marker(store: &Store) -> Oid<Sha1> {
    let empty = store.write_buf(Kind::Blob, b"").expect("write empty blob");
    write_tree(store, vec![("none", EntryKind::Blob, empty)])
}

/// Bind a value tree and a schema tree into a document root: the value's own
/// entries plus one reserved `.schema` entry.
fn bind(store: &Store, value: &Oid<Sha1>, schema: &Oid<Sha1>) -> Oid<Sha1> {
    let mut entries = store
        .get_tree(value)
        .expect("read value tree")
        .expect("value present");
    entries.push(TreeEntry {
        mode: EntryMode::from(EntryKind::Tree),
        filename: document::SCHEMA_ENTRY.into(),
        oid: schema.to_raw(),
    });
    entries.sort();
    store.write(&Tree { entries }).expect("write document")
}

/// A store holding a `Book` value and its schema, bound into a document root.
fn bound_document() -> (Store, Oid<Sha1>, Oid<Sha1>) {
    let store = Store::new();
    let book = Book {
        title: "Git Trees".into(),
        pages: 120,
    };
    let value = serialize_into(&book, &store).expect("serialize value");
    let schema = schema_of::<Book>().expect("derive schema");
    let schema = serialize_into(&schema, &store).expect("serialize schema");
    let root = bind(&store, &value, &schema);
    (store, root, schema)
}

#[test]
fn split_returns_the_root_and_the_embedded_schema() {
    let (store, root, schema) = bound_document();
    let (split_root, split_schema) = document::split(root.to_raw(), &store).expect("split");
    assert_eq!(split_root, root.to_raw());
    assert_eq!(split_schema, schema.to_raw());
}

#[test]
fn a_bare_value_tree_is_not_a_document() {
    let store = Store::new();
    let value = serialize_into(
        &Book {
            title: "Git Trees".into(),
            pages: 120,
        },
        &store,
    )
    .expect("serialize value");
    let err = document::split(value.to_raw(), &store).expect_err("no envelope");
    assert!(
        matches!(err, DocumentError::NotSubtreeBound { root, .. } if root == value.to_raw()),
        "{err:?}"
    );
}

#[test]
fn a_schema_entry_that_is_not_a_tree_is_rejected() {
    let store = Store::new();
    let value = serialize_into(
        &Book {
            title: "Git Trees".into(),
            pages: 120,
        },
        &store,
    )
    .expect("serialize value");
    let mut entries = store.get_tree(&value).unwrap().unwrap();
    let blob = store.write_buf(Kind::Blob, b"not a tree").unwrap();
    entries.push(TreeEntry {
        mode: EntryMode::from(EntryKind::Blob),
        filename: document::SCHEMA_ENTRY.into(),
        oid: blob.to_raw(),
    });
    entries.sort();
    let root = store.write(&Tree { entries }).unwrap();

    let err = document::split(root.to_raw(), &store).expect_err("schema is a blob");
    assert!(
        matches!(err, DocumentError::NotSubtreeBound { .. }),
        "{err:?}"
    );
}

#[test]
fn a_lone_schema_entry_is_not_a_document() {
    let store = Store::new();
    let schema = serialize_into(&schema_of::<Book>().unwrap(), &store).unwrap();
    let root = write_tree(
        &store,
        vec![(document::SCHEMA_ENTRY, EntryKind::Tree, schema)],
    );

    let err = document::split(root.to_raw(), &store).expect_err("no value entries");
    assert!(
        matches!(err, DocumentError::NotSubtreeBound { .. }),
        "{err:?}"
    );
}

#[test]
fn an_absent_root_is_a_missing_object() {
    let store = Store::new();
    let other = Store::new();
    let value = serialize_into(
        &Book {
            title: "Git Trees".into(),
            pages: 120,
        },
        &other,
    )
    .unwrap();

    let err = document::split(value.to_raw(), &store).expect_err("root is absent");
    assert!(
        matches!(err, DocumentError::MissingObject { oid } if oid == value.to_raw()),
        "{err:?}"
    );
}

#[test]
fn a_non_tree_root_is_not_a_document() {
    let store = Store::new();
    let blob = store.write_buf(Kind::Blob, b"value").unwrap();

    let err = document::split(blob.to_raw(), &store).expect_err("root is a blob");
    assert!(
        matches!(err, DocumentError::NotATree { oid } if oid == blob.to_raw()),
        "{err:?}"
    );
}

#[test]
fn a_missing_schema_object_names_the_subtree() {
    let store = Store::new();
    let value = serialize_into(
        &Book {
            title: "Git Trees".into(),
            pages: 120,
        },
        &store,
    )
    .unwrap();
    // The schema lives in a different store, so `.schema` names an absent object.
    let elsewhere = Store::new();
    let schema = serialize_into(&schema_of::<Book>().unwrap(), &elsewhere).unwrap();
    let root = bind(&store, &value, &schema);

    match document::split(root.to_raw(), &store) {
        Err(DocumentError::SubtreeMissing {
            subtree,
            oid,
            root: reported,
        }) => {
            assert_eq!(subtree, Subtree::Schema);
            assert_eq!(oid, schema.to_raw());
            assert_eq!(reported, root.to_raw());
        }
        other => panic!("expected SubtreeMissing, got {other:?}"),
    }
}

#[test]
fn the_unbound_view_hides_the_schema_entry() {
    let (store, root, _) = bound_document();
    let view = UnboundValue::new(root.to_raw(), &store).expect("view");

    let mut buf = Vec::new();
    let data = view
        .try_find(root.to_raw().as_ref(), &mut buf)
        .expect("find")
        .expect("present");
    let tree = TreeRef::from_bytes(data.data, data.object_hash).expect("decode");

    assert!(
        tree.entries
            .iter()
            .all(|entry| entry.filename != document::SCHEMA_ENTRY.as_bytes()),
        "the view must answer the root with the value's entries only"
    );
    assert!(!tree.entries.is_empty(), "the value has fields");
}

#[test]
fn read_value_decodes_a_document_through_the_view() -> anyhow::Result<()> {
    let (store, root, _) = bound_document();
    let schema = schema_of::<Book>()?;

    let book: Book = document::read_value(
        root.to_raw(),
        &schema,
        &store,
        |root, _doc, view| -> anyhow::Result<Book> {
            let root = Oid::new(*root)?;
            Ok(deserialize(&root, view)?)
        },
    )?;

    assert_eq!(
        book,
        Book {
            title: "Git Trees".into(),
            pages: 120
        }
    );
    Ok(())
}

#[test]
fn ensure_struct_root_accepts_a_struct_root() {
    let schema = schema_of::<Book>().expect("derive schema");
    document::ensure_struct_root(&schema).expect("a struct root is a document schema");
}

#[test]
fn ensure_struct_root_rejects_a_non_struct_root() {
    let schema = schema_of::<Vec<u32>>().expect("derive schema");
    match document::ensure_struct_root(&schema) {
        Err(DocumentError::DocumentRootNotStruct { kind, node }) => {
            assert_eq!(kind, "Vec");
            assert_eq!(node, "list");
        }
        other => panic!("expected DocumentRootNotStruct, got {other:?}"),
    }
}

/// A schema whose root `ref` names no definition can never resolve to a struct.
/// Only a decoded, unvalidated document can hold one, so it is reached by
/// deserializing a hand-built schema tree.
#[test]
fn ensure_struct_root_rejects_an_unresolved_root() {
    let store = Store::new();
    let name = store.write_buf(Kind::Blob, b"Missing\n").unwrap();
    let root = write_tree(&store, vec![("ref", EntryKind::Blob, name)]);
    let kind = store.write_buf(Kind::Blob, b"broken\n").unwrap();
    let tree = write_tree(
        &store,
        vec![
            ("defs", EntryKind::Tree, marker(&store)),
            ("kind", EntryKind::Blob, kind),
            ("root", EntryKind::Tree, root),
        ],
    );

    let schema: Schema = deserialize(&tree, &store).expect("decode the unvalidated schema");
    match document::ensure_struct_root(&schema) {
        Err(DocumentError::SchemaRootUnresolved { kind, source }) => {
            assert_eq!(kind, "broken");
            assert!(matches!(source, RefError::Dangling { name } if name == "Missing"));
        }
        other => panic!("expected SchemaRootUnresolved, got {other:?}"),
    }
}
