//! Integration tests for schema-driven deserialization
//! (`deserialize_value_with_schema` / `validate_with_schema`, `value`
//! feature).
//!
//! Covers spec requirement:
//!   deserialization.schema-driven
//!     — a schema document recovers a faithful `Value` from a typed tree
//!       without the original `Facet` type: numbers as numbers, bools as
//!       bools, enums as single-member tagged objects, `RawTree` as the hex
//!       object id, `Dynamic` via the bare heuristic, and `Ref` through the
//!       document's `defs` table.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use facet::Facet;
use facet_git_tree::{
    DeserializeError, EntryKind, EntryMode, Node, ObjectId, ObjectStore, RawTree, Schema,
    SchemaReadError, SchemaWriteError, TreeEntry, VariantKind, deserialize,
    deserialize_value_with_schema, schema_of, serialize, serialize_into,
    serialize_value_with_schema, validate_with_schema,
};
use facet_value::{VNumber, Value, value};
use gix_object::{Kind, Write as _};

mod common;
use common::{Event, Person, Point, TreeNode, WithDefault, WithMap, WithOptional};

// --- faithful scalars ---

/// A typed struct reads back with full fidelity: numbers are numbers and
/// bools are bools, not the strings the bare heuristic yields.
#[test]
fn person_reads_faithfully() -> anyhow::Result<()> {
    let person = Person {
        name: "Alice".into(),
        age: 30,
        active: true,
    };
    let (root, store) = serialize(&person)?;
    let doc = schema_of::<Person>()?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v, value!({ "name": "Alice", "age": 30, "active": true }));
    Ok(())
}

/// 128-bit extremes are preserved exactly, never routed through a float.
#[test]
fn extreme_128_bit_integers_are_exact() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct Big {
        a: u128,
        b: i128,
    }
    let (root, store) = serialize(&Big {
        a: u128::MAX,
        b: i128::MIN,
    })?;
    let doc = schema_of::<Big>()?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(
        v.as_object().unwrap().get("a").unwrap().as_number(),
        Some(&VNumber::from_u128(u128::MAX))
    );
    assert_eq!(
        v.as_object().unwrap().get("b").unwrap().as_number(),
        Some(&VNumber::from_i128(i128::MIN))
    );
    Ok(())
}

// --- Option ---

/// `None` reads as `NULL`; `Some` reads as the inner value directly.
#[test]
fn option_none_is_null_some_is_inner() -> anyhow::Result<()> {
    let doc = schema_of::<WithOptional>()?;

    let (root, store) = serialize(&WithOptional { maybe: None })?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v, value!({ "maybe": null }));

    let (root, store) = serialize(&WithOptional { maybe: Some(5) })?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v, value!({ "maybe": 5 }));
    Ok(())
}

// --- enums ---

/// Every variant kind reads as a single-member object tagged by the variant
/// name: unit → NULL, one-field tuple → inner, tuple → array, struct → object.
#[test]
fn enum_variants_read_as_tagged_objects() -> anyhow::Result<()> {
    let doc = schema_of::<Event>()?;
    let read = |e: &Event| -> anyhow::Result<Value> {
        let (root, store) = serialize(e)?;
        Ok(deserialize_value_with_schema(&root, &doc, &store)?)
    };

    assert_eq!(read(&Event::Ping)?, value!({ "Ping": null }));
    assert_eq!(
        read(&Event::Message("hi".into()))?,
        value!({ "Message": "hi" })
    );
    assert_eq!(read(&Event::Move(1, -2))?, value!({ "Move": [1, (-2)] }));
    assert_eq!(
        read(&Event::Login {
            user: "bob".into(),
            ok: true
        })?,
        value!({ "Login": { "user": "bob", "ok": true } })
    );
    Ok(())
}

/// A variant name absent from the schema is an `UnknownVariant` error.
#[test]
fn unknown_variant_errors() -> anyhow::Result<()> {
    let (root, store) = serialize(&Event::Ping)?;
    // A schema for the same enum, but without the `Ping` variant.
    let doc = Schema {
        kind: "example".into(),
        root: Node::Enum(Default::default()),
        defs: Default::default(),
    };
    let err = deserialize_value_with_schema(&root, &doc, &store).unwrap_err();
    assert!(
        matches!(&err, SchemaReadError::UnknownVariant { variant, .. } if variant == "Ping"),
        "expected UnknownVariant, got {err:?}"
    );
    Ok(())
}

// --- maps ---

/// A string-keyed map reads as an object keyed by those strings.
#[test]
fn scalar_key_map_reads_as_object() -> anyhow::Result<()> {
    let mut table = HashMap::new();
    table.insert("greeting".to_string(), "hello".to_string());
    let (root, store) = serialize(&WithMap { table })?;
    let doc = schema_of::<WithMap>()?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v, value!({ "table": { "greeting": "hello" } }));
    Ok(())
}

/// A non-string-key map is an opaque canonical CBOR blob, not a pair tree.
#[test]
fn composite_key_map_reads_as_opaque_cbor_and_reencodes() -> anyhow::Result<()> {
    let mut table: HashMap<(u8, u8), String> = HashMap::new();
    table.insert((3, 4), "cell".to_string());
    let (root, store) = serialize(&table)?;
    let doc = schema_of::<HashMap<(u8, u8), String>>()?;
    assert_eq!(doc.root, Node::Cbor);
    // { [3, 4] => "cell" }, RFC 8949 core-deterministic encoding.
    let bytes = [0xa1, 0x82, 0x03, 0x04, 0x64, b'c', b'e', b'l', b'l'];
    assert_eq!(store.get_blob(&root)?.as_deref(), Some(bytes.as_slice()));
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v.as_bytes().map(|b| b.as_slice()), Some(bytes.as_slice()));
    assert_eq!(serialize_value_with_schema(&v, &doc, &store)?, root);
    assert_eq!(
        deserialize::<HashMap<(u8, u8), String>>(&root, &store)?,
        table
    );
    Ok(())
}

/// A smart-pointer scalar key (`Arc<str>`) is classified scalar by
/// `schema_of` — matching what the encoder actually writes after
/// transparency collapse (`collapse_shape`) — rather than composite, which is
/// what the raw, uncollapsed `Def::Pointer` key shape would otherwise suggest.
#[test]
fn arc_str_key_map_schema_is_scalar_and_reads() -> anyhow::Result<()> {
    let mut table: HashMap<Arc<str>, u32> = HashMap::new();
    table.insert(Arc::from("hi"), 1);

    let doc = schema_of::<HashMap<Arc<str>, u32>>()?;
    assert_eq!(
        doc.root,
        Node::Map {
            key: Box::new(Node::String),
            value: Box::new(Node::U32),
        },
        "an Arc<str> key must be schema'd as its collapsed scalar (String), not composite"
    );

    let (root, store) = serialize(&table)?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v, value!({ "hi": 1 }));
    Ok(())
}

// --- recursion via Ref ---

/// A recursive type reads through its `Ref` definition at every level.
#[test]
fn recursive_tree_reads_via_ref() -> anyhow::Result<()> {
    let tree = TreeNode {
        value: 1,
        children: vec![TreeNode {
            value: 2,
            children: vec![],
        }],
    };
    let (root, store) = serialize(&tree)?;
    let doc = schema_of::<TreeNode>()?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(
        v,
        value!({ "value": 1, "children": [{ "value": 2, "children": [] }] })
    );
    Ok(())
}

/// A `Ref` naming no definition is an `UnknownRef` error.
#[test]
fn unknown_ref_errors() -> anyhow::Result<()> {
    let (root, store) = serialize(&42i32)?;
    let doc = Schema {
        kind: "example".into(),
        root: Node::Ref("nope".into()),
        defs: Default::default(),
    };
    let err = deserialize_value_with_schema(&root, &doc, &store).unwrap_err();
    assert!(
        matches!(&err, SchemaReadError::UnknownRef(name) if name == "nope"),
        "expected UnknownRef, got {err:?}"
    );
    Ok(())
}

// --- RawTree and Dynamic ---

/// A `RawTree` node reads as the referenced object id in lowercase hex.
#[test]
fn raw_tree_reads_as_hex_string() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct WithRaw {
        raw: RawTree,
    }
    let (inner, store) = serialize(&Point { x: 1.0, y: 2.0 })?;
    let root = serialize_into(
        &WithRaw {
            raw: RawTree::new(inner),
        },
        &store,
    )?;
    let doc = schema_of::<WithRaw>()?;
    let v = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(v, value!({ "raw": (inner.to_string()) }));
    Ok(())
}

/// Wrap a leaf in single-entry trees, from the innermost tree out.
fn nest_single(
    store: &ObjectStore,
    oid: ObjectId,
    wraps: usize,
    leaf_is_blob: bool,
    name: &str,
) -> ObjectId {
    let mut current = oid;
    let mut current_kind = if leaf_is_blob {
        EntryKind::Blob
    } else {
        EntryKind::Tree
    };
    for _ in 0..wraps {
        let entries = vec![TreeEntry {
            mode: EntryMode::from(current_kind),
            filename: name.into(),
            oid: current,
        }];
        current = store
            .write(&gix_object::Tree { entries })
            .expect("write tree");
        current_kind = EntryKind::Tree;
    }
    current
}

/// A `Node::Dynamic` node hands off to the same recursion-depth budget the
/// surrounding schema-driven read is already spending from, rather than
/// resetting it to `0`. Neither half of this tree — 20 `Node::Optional`
/// levels, then 20 more levels the dynamic heuristic itself must walk —
/// exceeds the codec depth bound (32) alone, but their sum (40) does, so the
/// tree is only rejected if the depth carries across the hand-off.
#[test]
fn dynamic_schema_node_shares_the_surrounding_depth_budget() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let leaf = store.write_buf(Kind::Blob, b"leaf\n").expect("write blob");
    // Escaped `%some` is a valid user key inside a dynamic object. Bare
    // `some` remains the structural option tag on the schema-driven side.
    let dynamic_part = nest_single(&store, leaf, 20, true, "%some");
    let root = nest_single(&store, dynamic_part, 20, false, "some");

    // Each half alone is valid and below the limit.
    let dynamic_doc = Schema {
        kind: "dynamic".into(),
        root: Node::Dynamic,
        defs: BTreeMap::new(),
    };
    deserialize_value_with_schema(&dynamic_part, &dynamic_doc, &store)?;
    let shallow_root = nest_single(&store, leaf, 20, true, "some");
    let mut shallow_schema = Node::String;
    for _ in 0..20 {
        shallow_schema = Node::Optional(Box::new(shallow_schema));
    }
    let shallow_doc = Schema {
        kind: "options".into(),
        root: shallow_schema,
        defs: BTreeMap::new(),
    };
    deserialize_value_with_schema(&shallow_root, &shallow_doc, &store)?;

    let mut schema = Node::Dynamic;
    for _ in 0..20 {
        schema = Node::Optional(Box::new(schema));
    }
    let doc = Schema {
        kind: "example".into(),
        root: schema,
        defs: Default::default(),
    };

    let err = deserialize_value_with_schema(&root, &doc, &store).unwrap_err();
    assert!(
        matches!(
            err,
            SchemaReadError::Deserialize(DeserializeError::MaxDepth(_))
        ),
        "expected MaxDepth from the combined schema + dynamic depth, got {err:?}"
    );
    Ok(())
}

/// A `Dynamic` node delegates to the bare heuristic: the same documented-lossy
/// reading a plain `deserialize::<Value>` produces.
#[test]
fn dynamic_delegates_to_heuristic() -> anyhow::Result<()> {
    let person = Person {
        name: "Alice".into(),
        age: 30,
        active: true,
    };
    let (root, store) = serialize(&person)?;
    let doc = Schema {
        kind: "example".into(),
        root: Node::Dynamic,
        defs: Default::default(),
    };
    let via_schema = deserialize_value_with_schema(&root, &doc, &store)?;
    let via_heuristic: Value = deserialize(&root, &store)?;
    assert_eq!(via_schema, via_heuristic);
    // Lossy as documented: the number came back as its textual form.
    assert_eq!(
        via_schema,
        value!({ "name": "Alice", "age": "30", "active": "true" })
    );
    Ok(())
}

// --- validation ---

/// `validate_with_schema` succeeds on a conforming tree and fails on a
/// non-conforming one.
#[test]
fn validate_ok_and_err() -> anyhow::Result<()> {
    let (root, store) = serialize(&Person {
        name: "Alice".into(),
        age: 30,
        active: true,
    })?;
    validate_with_schema(&root, &schema_of::<Person>()?, &store)?;

    // A Person tree is not the unit marker, so a Unit schema must reject it.
    let unit = Schema {
        kind: "example".into(),
        root: Node::Unit,
        defs: Default::default(),
    };
    let err = validate_with_schema(&root, &unit, &store).unwrap_err();
    assert!(
        matches!(err, SchemaReadError::MalformedUnit { found: 3 }),
        "expected MalformedUnit, got {err:?}"
    );
    Ok(())
}

/// A struct schema rejects a tree missing one of its fields, rather than
/// reading the field as absent.
#[test]
fn struct_read_requires_every_schema_field() -> anyhow::Result<()> {
    let (root, store) = serialize(&Point { x: 1.0, y: 2.0 })?;
    let mut doc = schema_of::<Point>()?;
    let Some(Node::Struct(fields)) = doc.defs.get_mut("Point") else {
        panic!("Point is a struct definition");
    };
    fields.insert("z".into(), Node::F64);

    let err = validate_with_schema(&root, &doc, &store).unwrap_err();
    assert!(
        matches!(&err, SchemaReadError::MissingField { field } if field == "z"),
        "expected MissingField, got {err:?}"
    );
    Ok(())
}

/// A defaulted field is still required by the schema and the typed reader.
#[test]
fn omitted_defaulted_field_is_rejected_on_write_and_read() -> anyhow::Result<()> {
    let doc = schema_of::<WithDefault>()?;
    let store = ObjectStore::default();
    let error = serialize_value_with_schema(&value!({ "label": "x" }), &doc, &store).unwrap_err();
    assert!(
        matches!(&error, SchemaWriteError::MissingField { field, .. } if field == "count"),
        "expected MissingField(count), got {error:?}"
    );

    let (root, store) = serialize(&WithDefault {
        label: "x".into(),
        count: 0,
    })?;
    let fields = common::tree_entries(&store, &root);
    assert_eq!(fields.len(), 2, "the typed writer must emit the default");
    let label = common::find_entry(&store, &root, "label");
    let missing = store
        .write(&gix_object::Tree {
            entries: vec![label],
        })
        .expect("write tree missing defaulted field");
    let error = validate_with_schema(&missing, &doc, &store).unwrap_err();
    assert!(
        matches!(&error, SchemaReadError::MissingField { field } if field == "count"),
        "expected MissingField(count), got {error:?}"
    );
    let error = deserialize::<WithDefault>(&missing, &store).unwrap_err();
    assert!(
        matches!(&error, DeserializeError::MissingField { field } if field == "count"),
        "expected typed MissingField(count), got {error:?}"
    );
    Ok(())
}

/// A struct schema rejects a tree carrying an entry it does not define, so a
/// tree that merely overlaps a schema does not pass as conforming.
#[test]
fn struct_read_rejects_entries_absent_from_the_schema() -> anyhow::Result<()> {
    let (root, store) = serialize(&Point { x: 1.0, y: 2.0 })?;
    let mut doc = schema_of::<Point>()?;
    let Some(Node::Struct(fields)) = doc.defs.get_mut("Point") else {
        panic!("Point is a struct definition");
    };
    fields.remove("y");

    let err = validate_with_schema(&root, &doc, &store).unwrap_err();
    assert!(
        matches!(&err, SchemaReadError::UnexpectedEntry { entry } if entry == "y"),
        "expected UnexpectedEntry, got {err:?}"
    );
    Ok(())
}

// --- frozen-format conformance ---

/// The schema's kind is metadata; changing it must not alter bare-value decoding.
#[test]
fn schema_kind_is_opaque_for_reads_and_writes() -> anyhow::Result<()> {
    let person = Person {
        name: "Alice".into(),
        age: 30,
        active: true,
    };
    let (root, store) = serialize(&person)?;
    let doc = schema_of::<Person>()?;
    let mut relabeled = doc.clone();
    relabeled.kind = "arbitrary/other kind".into();
    let read = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(
        deserialize_value_with_schema(&root, &relabeled, &store)?,
        read
    );
    assert_eq!(
        serialize_value_with_schema(&read, &relabeled, &store)?,
        root
    );
    assert_ne!(serialize(&doc)?.0, serialize(&relabeled)?.0);
    Ok(())
}

/// The same value has identical typed and schema-directed objects in both Git
/// hash formats, including escaped names, ordinals, markers and direct payloads.
#[test]
fn escaped_fields_and_variants_roundtrip_in_both_hash_formats() -> anyhow::Result<()> {
    #[derive(Debug, Facet, PartialEq)]
    #[repr(u8)]
    enum Task {
        #[facet(rename = "none")]
        Wait,
        #[facet(rename = "a/b")]
        Note(String),
        Pair(i32, bool),
        Named {
            #[facet(rename = ".value")]
            label: String,
        },
    }

    #[derive(Debug, Facet, PartialEq)]
    struct Record {
        #[facet(rename = ".schema")]
        title: String,
        #[facet(rename = "some")]
        maybe: Option<i32>,
        labels: BTreeMap<String, String>,
        numbers: Vec<i32>,
        bytes: Vec<u8>,
        task: Task,
    }

    for kind in [gix_hash::Kind::Sha1, gix_hash::Kind::Sha256] {
        for task in [
            Task::Wait,
            Task::Note("hi\n".into()),
            Task::Pair(-3, true),
            Task::Named { label: "x".into() },
        ] {
            let record = Record {
                title: "T\n".into(),
                maybe: Some(7),
                labels: BTreeMap::from([
                    ("none".into(), "n".into()),
                    ("a/b".into(), "slash".into()),
                    ("".into(), "empty".into()),
                ]),
                numbers: (0..17).collect(),
                bytes: vec![0, 0xff, b'\n'],
                task,
            };
            let doc = schema_of::<Record>()?;
            let store = ObjectStore::with_hash_kind(kind);
            let root = serialize_into(&record, &store)?;
            let title = common::find_entry(&store, &root, "%.schema");
            assert_eq!(store.get_blob(&title.oid)?.as_deref(), Some(&b"T\n\n"[..]));
            let maybe = common::find_entry(&store, &root, "%some");
            assert_eq!(maybe.mode.kind(), EntryKind::Tree);
            let some = common::find_entry(&store, &maybe.oid, "some");
            assert_eq!(store.get_blob(&some.oid)?.as_deref(), Some(&b"7\n"[..]));
            let labels = common::find_entry(&store, &root, "labels");
            for name in ["%", "%none", "%a%2Fb"] {
                common::find_entry(&store, &labels.oid, name);
            }
            let numbers = common::find_entry(&store, &root, "numbers");
            let sixteenth = common::find_entry(&store, &numbers.oid, "010");
            assert_eq!(
                store.get_blob(&sixteenth.oid)?.as_deref(),
                Some(&b"16\n"[..])
            );
            let bytes = common::find_entry(&store, &root, "bytes");
            assert_eq!(
                store.get_blob(&bytes.oid)?.as_deref(),
                Some(&b"\0\xff\n"[..])
            );
            let task = common::find_entry(&store, &root, "task");
            match &record.task {
                Task::Wait => {
                    assert_eq!(task.mode.kind(), EntryKind::Blob);
                    assert_eq!(store.get_blob(&task.oid)?.as_deref(), Some(&b"%none\n"[..]));
                }
                Task::Note(_) => {
                    let note = common::find_entry(&store, &task.oid, "%a%2Fb");
                    assert_eq!(note.mode.kind(), EntryKind::Blob);
                    assert_eq!(store.get_blob(&note.oid)?.as_deref(), Some(&b"hi\n\n"[..]));
                }
                Task::Pair(_, _) => {
                    let pair = common::find_entry(&store, &task.oid, "Pair");
                    assert_eq!(pair.mode.kind(), EntryKind::Tree);
                    common::find_entry(&store, &pair.oid, "000");
                    common::find_entry(&store, &pair.oid, "001");
                }
                Task::Named { .. } => {
                    let named = common::find_entry(&store, &task.oid, "Named");
                    assert_eq!(named.mode.kind(), EntryKind::Tree);
                    common::find_entry(&store, &named.oid, "%.value");
                }
            }
            let value = deserialize_value_with_schema(&root, &doc, &store)?;
            validate_with_schema(&root, &doc, &store)?;
            assert_eq!(serialize_value_with_schema(&value, &doc, &store)?, root);
            assert_eq!(deserialize::<Record>(&root, &store)?, record);
        }
    }
    Ok(())
}

#[test]
fn options_and_empty_containers_roundtrip_via_marker() -> anyhow::Result<()> {
    #[derive(Debug, Facet, PartialEq)]
    struct Containers {
        maybe: Option<i32>,
        list: Vec<i32>,
        map: BTreeMap<String, String>,
        unit: (),
    }
    let original = Containers {
        maybe: None,
        list: vec![],
        map: BTreeMap::new(),
        unit: (),
    };
    let (root, store) = serialize(&original)?;
    for name in ["maybe", "list", "map", "unit"] {
        let field = common::find_entry(&store, &root, name);
        let entries = common::tree_entries(&store, &field.oid);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "none");
        assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
        assert_eq!(store.get_blob(&entries[0].oid)?.as_deref(), Some(&b""[..]));
    }
    let doc = schema_of::<Containers>()?;
    let decoded = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(
        decoded,
        value!({ "maybe": null, "list": [], "map": {}, "unit": null })
    );
    assert_eq!(serialize_value_with_schema(&decoded, &doc, &store)?, root);
    assert_eq!(deserialize::<Containers>(&root, &store)?, original);
    Ok(())
}

/// Validate canonical CBOR, including inputs that a permissive parser could
/// decode but whose bytes would not survive re-encoding unchanged.
#[test]
fn schema_cbor_requires_one_canonical_item() -> anyhow::Result<()> {
    let doc = Schema {
        kind: "opaque".into(),
        root: Node::Cbor,
        defs: BTreeMap::new(),
    };
    let store = ObjectStore::default();
    for bytes in [
        vec![0x00],
        vec![0x0a],
        vec![0xa1, 0x82, 0x03, 0x04, 0x64, b'c', b'e', b'l', b'l'],
    ] {
        let root = store.write_buf(Kind::Blob, &bytes).expect("write blob");
        let value = deserialize_value_with_schema(&root, &doc, &store)?;
        assert_eq!(
            value.as_bytes().map(|b| b.as_slice()),
            Some(bytes.as_slice())
        );
        validate_with_schema(&root, &doc, &store)?;
        assert_eq!(serialize_value_with_schema(&value, &doc, &store)?, root);
        assert_eq!(
            serialize_value_with_schema(&Value::from(bytes.clone()), &doc, &store)?,
            root
        );
    }
    for bytes in [
        vec![],
        vec![0x18, 0x00],                   // zero in a non-minimal integer encoding
        vec![0x00, 0x00],                   // trailing item
        vec![0x9f, 0x01, 0xff],             // indefinite-length array
        vec![0xa2, 0x02, 0x00, 0x01, 0x00], // unordered map keys
    ] {
        let root = store.write_buf(Kind::Blob, &bytes).expect("write blob");
        assert!(
            deserialize_value_with_schema(&root, &doc, &store).is_err(),
            "accepted non-canonical CBOR {bytes:02x?}"
        );
        assert!(
            validate_with_schema(&root, &doc, &store).is_err(),
            "CBOR {bytes:02x?}"
        );
        assert!(
            serialize_value_with_schema(&Value::from(bytes.clone()), &doc, &store).is_err(),
            "wrote non-canonical CBOR {bytes:02x?}"
        );
    }
    let (tree, tree_store) = serialize(&())?;
    assert!(deserialize_value_with_schema(&tree, &doc, &tree_store).is_err());
    Ok(())
}

/// Schema validation must not depend on whether a bad definition or enum
/// branch happens to be reached by this value.
#[test]
fn invalid_schema_is_rejected_on_read_and_write_before_value_walk() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let root = store.write_buf(Kind::Blob, b"ok\n").expect("write blob");
    let schema = |root: Node, defs: BTreeMap<String, Node>| Schema {
        kind: "test".into(),
        root,
        defs,
    };
    let invalid_key = Node::Map {
        key: Box::new(Node::I32),
        value: Box::new(Node::String),
    };
    let unused_variant = Node::Enum(BTreeMap::from([(
        "Unused".into(),
        VariantKind::Tuple(vec![Node::Ref("missing".into())]),
    )]));
    let cases = [
        (
            "unused non-string map key",
            schema(
                Node::String,
                BTreeMap::from([("Unused".into(), invalid_key)]),
            ),
        ),
        (
            "unused referenced key, not literally String",
            schema(
                Node::String,
                BTreeMap::from([
                    ("K".into(), Node::String),
                    (
                        "Unused".into(),
                        Node::Map {
                            key: Box::new(Node::Ref("K".into())),
                            value: Box::new(Node::String),
                        },
                    ),
                ]),
            ),
        ),
        (
            "dangling root ref",
            schema(Node::Ref("missing".into()), BTreeMap::new()),
        ),
        (
            "dangling inactive variant",
            schema(
                Node::String,
                BTreeMap::from([("Unused".into(), unused_variant)]),
            ),
        ),
        (
            "direct ref cycle",
            schema(
                Node::String,
                BTreeMap::from([("A".into(), Node::Ref("A".into()))]),
            ),
        ),
        (
            "mutual unguarded struct cycle",
            schema(
                Node::String,
                BTreeMap::from([
                    (
                        "A".into(),
                        Node::Struct(BTreeMap::from([("child".into(), Node::Ref("B".into()))])),
                    ),
                    ("B".into(), Node::Tuple(vec![Node::Ref("A".into())])),
                ]),
            ),
        ),
        (
            "oversized array",
            schema(
                Node::String,
                BTreeMap::from([(
                    "Unused".into(),
                    Node::Array {
                        elem: Box::new(Node::I32),
                        len: 4097,
                    },
                )]),
            ),
        ),
        (
            "oversized tuple",
            schema(
                Node::String,
                BTreeMap::from([("Unused".into(), Node::Tuple(vec![Node::Unit; 4097]))]),
            ),
        ),
    ];
    for (case, doc) in cases {
        assert!(
            deserialize_value_with_schema(&root, &doc, &store).is_err(),
            "accepted {case}"
        );
        assert!(
            validate_with_schema(&root, &doc, &store).is_err(),
            "validated {case}"
        );
        assert!(
            serialize_value_with_schema(&value!("ok"), &doc, &store).is_err(),
            "wrote with {case}"
        );
    }
    Ok(())
}

#[test]
fn guarded_recursive_schemas_and_aliases_are_productive() -> anyhow::Result<()> {
    let doc = Schema {
        kind: "recursive".into(),
        root: Node::Ref("Alias".into()),
        defs: BTreeMap::from([
            ("Alias".into(), Node::Ref("Node".into())),
            (
                "Node".into(),
                Node::Struct(BTreeMap::from([
                    ("value".into(), Node::I32),
                    (
                        "next".into(),
                        Node::Optional(Box::new(Node::Ref("Alias".into()))),
                    ),
                ])),
            ),
        ]),
    };
    let original = value!({ "value": 1, "next": { "value": 2, "next": null } });
    let store = ObjectStore::default();
    let root = serialize_value_with_schema(&original, &doc, &store)?;
    validate_with_schema(&root, &doc, &store)?;
    let read = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(read, original);
    assert_eq!(serialize_value_with_schema(&read, &doc, &store)?, root);
    Ok(())
}

#[test]
fn map_keys_must_be_literal_string_even_at_root() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let root = serialize_into(&BTreeMap::<String, String>::new(), &store)?;
    for key in [Node::I32, Node::Ref("K".into())] {
        let doc = Schema {
            kind: "map".into(),
            root: Node::Map {
                key: Box::new(key),
                value: Box::new(Node::String),
            },
            defs: BTreeMap::from([("K".into(), Node::String)]),
        };
        assert!(deserialize_value_with_schema(&root, &doc, &store).is_err());
        assert!(validate_with_schema(&root, &doc, &store).is_err());
        assert!(serialize_value_with_schema(&value!({}), &doc, &store).is_err());
    }
    Ok(())
}

#[test]
fn schema_writer_uses_git_tree_order_for_blob_tree_prefixes() -> anyhow::Result<()> {
    let doc = Schema {
        kind: "order".into(),
        root: Node::Struct(BTreeMap::from([
            ("a".into(), Node::List(Box::new(Node::I32))),
            ("a.b".into(), Node::String),
        ])),
        defs: BTreeMap::new(),
    };
    let value = value!({ "a": [1], "a.b": "x" });
    let store = ObjectStore::default();
    let root = serialize_value_with_schema(&value, &doc, &store)?;
    let entries = common::tree_entries(&store, &root);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].filename, "a.b");
    assert_eq!(entries[0].mode.kind(), EntryKind::Blob);
    assert_eq!(entries[1].filename, "a");
    assert_eq!(entries[1].mode.kind(), EntryKind::Tree);
    let decoded = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(decoded, value);
    assert_eq!(serialize_value_with_schema(&decoded, &doc, &store)?, root);
    Ok(())
}

#[test]
fn schema_directed_floats_have_canonical_spelling_and_fixed_points() -> anyhow::Result<()> {
    for (number, bytes) in [
        (f32::from_bits(0x4895_1f84), b"305404.12\n".as_slice()),
        (-0.0f32, b"0.0\n"),
        (f32::NAN, b"nan\n"),
        (f32::INFINITY, b"inf\n"),
        (f32::NEG_INFINITY, b"-inf\n"),
    ] {
        let (root, store) = serialize(&number)?;
        assert_eq!(store.get_blob(&root)?.as_deref(), Some(bytes));
        let doc = schema_of::<f32>()?;
        let value = deserialize_value_with_schema(&root, &doc, &store)?;
        assert_eq!(serialize_value_with_schema(&value, &doc, &store)?, root);
    }
    for (number, bytes) in [
        (1e21f64, b"1e21\n".as_slice()),
        (-0.0f64, b"0.0\n"),
        (f64::NAN, b"nan\n"),
        (f64::INFINITY, b"inf\n"),
        (f64::NEG_INFINITY, b"-inf\n"),
    ] {
        let (root, store) = serialize(&number)?;
        assert_eq!(store.get_blob(&root)?.as_deref(), Some(bytes));
        let doc = schema_of::<f64>()?;
        let value = deserialize_value_with_schema(&root, &doc, &store)?;
        assert_eq!(serialize_value_with_schema(&value, &doc, &store)?, root);
    }
    Ok(())
}

// Raw Git tree entries allow malformed modes, order, and names to reach the reader.
type ForeignEntry<'a> = (&'a [u8], &'a [u8], ObjectId);

fn foreign_tree(store: &ObjectStore, entries: &[ForeignEntry<'_>]) -> ObjectId {
    let mut bytes = Vec::new();
    for (mode, name, oid) in entries {
        bytes.extend_from_slice(mode);
        bytes.push(b' ');
        bytes.extend_from_slice(name);
        bytes.push(0);
        bytes.extend_from_slice(oid.as_bytes());
    }
    store
        .write_buf(Kind::Tree, &bytes)
        .expect("write foreign tree")
}

fn assert_schema_read_rejects(store: &ObjectStore, root: &ObjectId, node: Node, case: &str) {
    let doc = Schema {
        kind: "foreign".into(),
        root: node,
        defs: BTreeMap::new(),
    };
    assert!(
        deserialize_value_with_schema(root, &doc, store).is_err(),
        "accepted {case}"
    );
    assert!(
        validate_with_schema(root, &doc, store).is_err(),
        "validated {case}"
    );
}

#[test]
fn foreign_schema_directed_names_ordinals_modes_and_markers_are_rejected() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let leaf = store.write_buf(Kind::Blob, b"x\n").expect("write blob");
    let empty = store.write_buf(Kind::Blob, b"").expect("write blob");
    let map = Node::Map {
        key: Box::new(Node::String),
        value: Box::new(Node::String),
    };
    for name in [
        b".schema".as_slice(),
        b".value",
        b"none",
        b"some",
        b"%74itle",
        b"%a%2fb",
        b"%FF",
    ] {
        let root = foreign_tree(&store, &[(b"100644", name, leaf)]);
        assert_schema_read_rejects(&store, &root, map.clone(), &format!("map name {name:?}"));
    }
    for name in [b"0".as_slice(), b"0000", b"00A", b"001"] {
        let root = foreign_tree(&store, &[(b"100644", name, leaf)]);
        assert_schema_read_rejects(
            &store,
            &root,
            Node::List(Box::new(Node::String)),
            &format!("ordinal {name:?}"),
        );
    }
    let array = foreign_tree(&store, &[(b"100644", b"000", leaf)]);
    assert_schema_read_rejects(
        &store,
        &array,
        Node::Array {
            elem: Box::new(Node::String),
            len: 2,
        },
        "array length mismatch",
    );
    for entries in [
        vec![(b"100755".as_slice(), b"a".as_slice(), leaf)],
        vec![(b"40000".as_slice(), b"a".as_slice(), leaf)],
        vec![
            (b"100644".as_slice(), b"z".as_slice(), leaf),
            (b"100644", b"a", leaf),
        ],
        vec![
            (b"100644".as_slice(), b"a".as_slice(), leaf),
            (b"100644", b"a", leaf),
        ],
    ] {
        let root = foreign_tree(&store, &entries);
        assert_schema_read_rejects(&store, &root, map.clone(), "mode, order, or duplicate");
    }
    let bad_none = foreign_tree(&store, &[(b"100644", b"none", leaf)]);
    let bad_some = foreign_tree(
        &store,
        &[(b"100644", b"some", leaf), (b"100644", b"z", leaf)],
    );
    let empty_tree = foreign_tree(&store, &[]);
    for (root, case) in [
        (bad_none, "nonempty marker blob"),
        (bad_some, "multiple option entries"),
        (empty_tree, "literal empty tree"),
    ] {
        assert_schema_read_rejects(&store, &root, Node::Optional(Box::new(Node::String)), case);
    }
    let none_with_wrong_kind = foreign_tree(&store, &[(b"40000", b"none", empty)]);
    assert_schema_read_rejects(
        &store,
        &none_with_wrong_kind,
        Node::Optional(Box::new(Node::String)),
        "marker entry has tree mode",
    );
    Ok(())
}

#[test]
fn foreign_schema_directed_scalar_leaves_must_be_canonical() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    for (node, bytes) in [
        (Node::String, b"no newline".as_slice()),
        (Node::Bool, b"True\n"),
        (Node::I32, b"01\n"),
        (Node::I32, b"-0\n"),
        (Node::U32, b"4294967296\n"),
        (Node::F32, b"1e39\n"),
        (Node::F64, b"-0.0\n"),
        (Node::F64, b"NaN\n"),
        (Node::Char, b"ab\n"),
    ] {
        let root = store.write_buf(Kind::Blob, bytes).expect("write blob");
        assert_schema_read_rejects(&store, &root, node, &format!("scalar {bytes:?}"));
    }
    Ok(())
}

#[test]
fn raw_tree_content_is_opaque_even_with_nonstandard_modes_or_empty_tree() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct WithRaw {
        raw: RawTree,
    }
    let store = ObjectStore::default();
    let target = store.write_buf(Kind::Blob, b"target").expect("write blob");
    let symlink_tree = foreign_tree(&store, &[(b"120000", b"link", target)]);
    let empty_tree = foreign_tree(&store, &[]);
    let doc = schema_of::<WithRaw>()?;
    for raw in [symlink_tree, empty_tree] {
        let root = serialize_into(
            &WithRaw {
                raw: RawTree::new(raw),
            },
            &store,
        )?;
        let decoded = deserialize_value_with_schema(&root, &doc, &store)?;
        assert_eq!(decoded, value!({ "raw": (raw.to_string()) }));
        validate_with_schema(&root, &doc, &store)?;
        assert_eq!(serialize_value_with_schema(&decoded, &doc, &store)?, root);
    }
    Ok(())
}

#[test]
fn contiguous_ordinal_like_dynamic_keys_still_reencode() -> anyhow::Result<()> {
    let original = value!({ "000": "x", "001": "y" });
    let doc = Schema {
        kind: "dynamic".into(),
        root: Node::Dynamic,
        defs: BTreeMap::new(),
    };
    let (root, store) = serialize(&original)?;
    let via_schema = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(
        serialize_value_with_schema(&via_schema, &doc, &store)?,
        root
    );
    let via_typed: Value = deserialize(&root, &store)?;
    assert_eq!(serialize_into(&via_typed, &store)?, root);
    Ok(())
}

#[test]
fn schema_driven_dynamic_object_with_noncontiguous_ordinal_keys_roundtrips() -> anyhow::Result<()> {
    let original = value!({ "000": "x", "002": "y" });
    let doc = Schema {
        kind: "dynamic".into(),
        root: Node::Dynamic,
        defs: BTreeMap::new(),
    };
    let (root, store) = serialize(&original)?;
    let typed_as_map: BTreeMap<String, String> = deserialize(&root, &store)?;
    assert_eq!(
        typed_as_map,
        BTreeMap::from([("000".into(), "x".into()), ("002".into(), "y".into())])
    );
    assert_eq!(serialize_value_with_schema(&original, &doc, &store)?, root);
    let read = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(serialize_value_with_schema(&read, &doc, &store)?, root);
    Ok(())
}

#[test]
fn typed_dynamic_object_with_noncontiguous_ordinal_keys_roundtrips() -> anyhow::Result<()> {
    let original = value!({ "000": "x", "002": "y" });
    let (root, store) = serialize(&original)?;
    let typed_as_map: BTreeMap<String, String> = deserialize(&root, &store)?;
    assert_eq!(
        typed_as_map,
        BTreeMap::from([("000".into(), "x".into()), ("002".into(), "y".into())])
    );
    let read: Value = deserialize(&root, &store)?;
    assert_eq!(serialize_into(&read, &store)?, root);
    Ok(())
}

#[test]
fn empty_struct_variant_and_single_tuple_unit_are_distinct() -> anyhow::Result<()> {
    #[derive(Debug, Facet, PartialEq)]
    #[repr(u8)]
    enum Event {
        Empty {},
        Unit(()),
        List(Vec<i32>),
    }
    for (event, expected) in [
        (Event::Empty {}, value!({ "Empty": {} })),
        (Event::Unit(()), value!({ "Unit": null })),
        (Event::List(vec![]), value!({ "List": [] })),
    ] {
        let doc = schema_of::<Event>()?;
        let (root, store) = serialize(&event)?;
        let value = deserialize_value_with_schema(&root, &doc, &store)?;
        assert_eq!(value, expected);
        assert_eq!(serialize_value_with_schema(&value, &doc, &store)?, root);
        assert_eq!(deserialize::<Event>(&root, &store)?, event);
    }
    Ok(())
}

#[test]
fn foreign_schema_directed_enum_tags_and_payloads_are_rejected() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let leaf = store.write_buf(Kind::Blob, b"x\n").expect("blob");
    let empty = store.write_buf(Kind::Blob, b"").expect("blob");
    let marker = foreign_tree(&store, &[(b"100644", b"none", empty)]);
    let variants = BTreeMap::from([
        ("Unit".into(), VariantKind::Unit),
        ("Direct".into(), VariantKind::Tuple(vec![Node::String])),
        (
            "Struct".into(),
            VariantKind::Struct(BTreeMap::from([("x".into(), Node::String)])),
        ),
    ]);
    let node = Node::Enum(variants);
    for bytes in [b"Unknown\n".as_slice(), b"Unit", b"Direct\n", b"%55nit\n"] {
        let root = store.write_buf(Kind::Blob, bytes).expect("blob");
        assert_schema_read_rejects(&store, &root, node.clone(), &format!("tag {bytes:?}"));
    }
    for entries in [
        vec![(b"40000".as_slice(), b"Unit".as_slice(), marker)],
        vec![(b"100644".as_slice(), b"Struct".as_slice(), leaf)],
        vec![(b"40000".as_slice(), b"Direct".as_slice(), marker)],
        vec![
            (b"100644".as_slice(), b"Direct".as_slice(), leaf),
            (b"40000", b"Struct", marker),
        ],
    ] {
        let root = foreign_tree(&store, &entries);
        assert_schema_read_rejects(&store, &root, node.clone(), "enum payload or tag count");
    }
    Ok(())
}

#[test]
fn empty_container_markers_must_be_singletons() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let empty = store.write_buf(Kind::Blob, b"").expect("blob");
    let value = store.write_buf(Kind::Blob, b"x\n").expect("blob");
    let marker_and_field = foreign_tree(
        &store,
        &[(b"100644", b"none", empty), (b"100644", b"x", value)],
    );
    for node in [
        Node::Unit,
        Node::Struct(BTreeMap::new()),
        Node::List(Box::new(Node::String)),
        Node::Map {
            key: Box::new(Node::String),
            value: Box::new(Node::String),
        },
        Node::Optional(Box::new(Node::String)),
    ] {
        assert_schema_read_rejects(&store, &marker_and_field, node, "marker plus another entry");
    }
    Ok(())
}

#[test]
fn typed_dynamic_missing_newline_must_not_be_accepted() -> anyhow::Result<()> {
    let store = ObjectStore::default();
    let doc = schema_of::<Value>()?;
    for bytes in [b"unterminated".as_slice(), b"\xff\x00", b""] {
        let root = store.write_buf(Kind::Blob, bytes).expect("blob");
        assert!(deserialize_value_with_schema(&root, &doc, &store).is_err());
        let read = deserialize::<Value>(&root, &store);
        assert!(
            read.is_err(),
            "accepted dynamic text leaf without newline: {bytes:?}"
        );
    }
    Ok(())
}

#[test]
fn typed_dynamic_binary_leaf_with_newline_is_a_fixed_point() -> anyhow::Result<()> {
    let source = Value::from(vec![0xff, b'\n', 0]);
    let (root, store) = serialize(&source)?;
    assert_eq!(store.get_blob(&root)?.as_deref(), Some(&b"\xff\n\0\n"[..]));
    let typed: Value = deserialize(&root, &store)?;
    assert_eq!(typed, source);
    assert_eq!(serialize_into(&typed, &store)?, root);
    let doc = schema_of::<Value>()?;
    let guided = deserialize_value_with_schema(&root, &doc, &store)?;
    assert_eq!(guided, source);
    assert_eq!(serialize_value_with_schema(&guided, &doc, &store)?, root);
    Ok(())
}
