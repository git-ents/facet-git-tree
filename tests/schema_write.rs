//! Integration tests for schema-directed serialization
//! (`serialize_value_with_schema`, `value` feature).
//!
//! Covers spec requirement:
//!   serialization.schema-directed
//!     — a schema document encodes a dynamic `Value` to the same objects the
//!       equivalent typed value produces (byte-identical, thus same oid);
//!       encoding validates schema and value, with errors naming the path;
//!       representable schema-read values and the JSON bridges reproduce the
//!       typed objects under the frozen format.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;

use facet::Facet;
use facet_git_tree::{
    EntryKind, Node, ObjectStore, RawTree, Schema, SchemaReadError, SchemaWriteError, VariantKind,
    deserialize_value_with_schema, schema_of, serialize, serialize_into,
    serialize_value_with_schema,
};
use facet_value::{VArray, VObject, Value, value};
use gix_object::Kind;

mod common;
use common::{
    Event, Person, Point, TreeNode, WithArray, WithDefault, WithMap, WithOptional, WithVec,
    checked, find_entry, typed,
};

/// The typed encoding's root oid: the ground truth every schema-directed write
/// must reproduce.
fn typed_root<T: for<'a> Facet<'a>>(value: &T) -> facet_git_tree::ObjectId {
    serialize(value).expect("typed serialize").0.into_raw()
}

fn schema(root: Node) -> Schema {
    Schema::new("test".into(), root, BTreeMap::new()).expect("valid schema")
}

fn one_field(name: &str, node: Node) -> BTreeMap<String, Node> {
    BTreeMap::from([(name.to_owned(), node)])
}

fn assert_marker(store: &ObjectStore, root: &facet_git_tree::ObjectId) {
    let entries = store
        .get_tree(&typed(root))
        .expect("read marker")
        .expect("tree");
    assert_eq!(entries.len(), 1, "marker must have exactly one entry");
    let marker = &entries[0];
    assert_eq!(marker.filename, "none");
    assert_eq!(marker.mode.kind(), EntryKind::Blob);
    assert_eq!(
        store
            .get_blob(&typed(&marker.oid))
            .expect("read object")
            .expect("blob"),
        b""
    );
}

/// Every value the schema-driven read can produce must re-encode to the exact
/// object it was read from — the round-trip property at the heart of the
/// accepted-set contract.
fn assert_reencodes<T>(value: T)
where
    T: for<'a> Facet<'a> + PartialEq + Debug,
{
    let (root, store) = serialize(&value).expect("typed serialize");
    let doc = schema_of::<T>().expect("schema_of");
    let read = deserialize_value_with_schema(&root, &checked(&doc), &store).expect("read");
    let out = ObjectStore::default();
    let written = serialize_value_with_schema(&read, &checked(&doc), &out).expect("schema write");
    assert_eq!(
        written, root,
        "re-encoding the read image of {value:?} must reproduce the typed oid"
    );
}

// --- the read image re-encodes identically ---

#[test]
fn reencodes_scalars_structs_and_collections() {
    assert_reencodes(Person {
        name: "Alice".into(),
        age: 30,
        active: true,
    });
    assert_reencodes(Point { x: 1.5, y: -2.25 });
    assert_reencodes(WithVec {
        items: vec![1, 2, 3],
    });
    assert_reencodes(WithArray {
        values: [10, 20, 30, 40],
    });
    assert_reencodes(WithOptional { maybe: Some(7) });
    assert_reencodes(WithOptional { maybe: None });
    assert_reencodes(TreeNode {
        value: 1,
        children: vec![TreeNode {
            value: 2,
            children: vec![],
        }],
    });
}

#[test]
fn reencodes_every_enum_variant() {
    assert_reencodes(Event::Ping);
    assert_reencodes(Event::Message("hi".into()));
    assert_reencodes(Event::Move(1, -2));
    assert_reencodes(Event::Login {
        user: "bob".into(),
        ok: true,
    });
}

#[test]
fn reencodes_scalar_key_map() {
    let mut table = HashMap::new();
    table.insert("greeting".to_string(), "hello".to_string());
    assert_reencodes(WithMap { table });
}

#[test]
fn composite_key_maps_use_canonical_cbor_and_preserve_their_read_image() {
    let table = BTreeMap::from([
        ((1u8, 2u8), "other".to_owned()),
        ((3, 4), "cell".to_owned()),
    ]);
    let doc = schema_of::<BTreeMap<(u8, u8), String>>().unwrap();
    assert!(matches!(doc.root(), Node::Cbor));

    // { [1, 2]: "other", [3, 4]: "cell" } in RFC 8949 core-deterministic form.
    let canonical = b"\xa2\x82\x01\x02\x65other\x82\x03\x04\x64cell";
    let (root, store) = serialize(&table).expect("non-string maps require canonical CBOR");
    assert_eq!(store.get_blob(&root).unwrap().unwrap(), canonical);
    let read = deserialize_value_with_schema(&root, &checked(&doc), &store).expect("read CBOR map");
    let out = ObjectStore::default();
    let written =
        serialize_value_with_schema(&read, &checked(&doc), &out).expect("re-encode CBOR map");
    assert_eq!(written, root);
    assert_eq!(out.get_blob(&written).unwrap().unwrap(), canonical);
}

#[test]
fn cbor_read_image_round_trips_and_rejects_noncanonical_bytes() {
    let doc = schema(Node::Cbor);
    let store = ObjectStore::default();
    for canonical in [b"\x01".as_slice(), b"\xa1\x61x\x01".as_slice()] {
        let root = store.write_buf(Kind::Blob, canonical).unwrap();
        let read = deserialize_value_with_schema(&root, &checked(&doc), &store)
            .expect("read canonical CBOR");
        assert_eq!(read.as_bytes().unwrap().as_slice(), canonical);
        let out = ObjectStore::default();
        let written =
            serialize_value_with_schema(&read, &checked(&doc), &out).expect("re-encode CBOR");
        assert_eq!(written, root);
        assert_eq!(out.get_blob(&written).unwrap().unwrap(), canonical);
    }
    for invalid in [b"\x18\x01".as_slice(), b"\x01\x00".as_slice()] {
        let root = store.write_buf(Kind::Blob, invalid).unwrap();
        assert!(
            deserialize_value_with_schema(&root, &checked(&doc), &store).is_err(),
            "non-canonical or trailing CBOR must be rejected: {invalid:?}"
        );
        assert!(
            serialize_value_with_schema(&Value::from(invalid.to_vec()), &checked(&doc), &store)
                .is_err(),
            "non-canonical or trailing CBOR must not be written: {invalid:?}"
        );
    }
}

#[test]
fn reencodes_128_bit_extremes() {
    #[derive(Debug, Facet, PartialEq)]
    struct Big {
        a: u128,
        b: i128,
    }
    assert_reencodes(Big {
        a: u128::MAX,
        b: i128::MIN,
    });
}

#[test]
fn reencodes_byte_sequence() {
    #[derive(Debug, Facet, PartialEq)]
    struct WithBytes {
        raw: Vec<u8>,
    }
    assert_reencodes(WithBytes {
        raw: vec![0, 1, 2, 255],
    });
}

// --- the JSON bridges (values outside the read image) ---

/// An integer supplied for an `f64` field is the canonical width-selection
/// bridge: JSON `1` must encode as the typed `f64` `1.0` would.
#[test]
fn integer_into_float_field_matches_typed() {
    let doc = schema_of::<Point>().unwrap();
    let store = ObjectStore::default();
    let written =
        serialize_value_with_schema(&value!({ "x": 1, "y": 2 }), &checked(&doc), &store).unwrap();
    assert_eq!(written, typed_root(&Point { x: 1.0, y: 2.0 }));
}

/// A string supplied for a `Bytes` field encodes as its UTF-8 bytes — the same
/// blob a typed `Vec<u8>` of those bytes produces.
#[test]
fn string_into_bytes_field_matches_typed() {
    #[derive(Facet)]
    struct WithBytes {
        raw: Vec<u8>,
    }
    let doc = schema_of::<WithBytes>().unwrap();
    let store = ObjectStore::default();
    let written =
        serialize_value_with_schema(&value!({ "raw": "hello" }), &checked(&doc), &store).unwrap();
    assert_eq!(
        written,
        typed_root(&WithBytes {
            raw: b"hello".to_vec()
        })
    );
}

/// A bare `Some` payload (an unwrapped value, as JSON yields) is normalized
/// into the `some`-wrapped tree the typed `Option` writes — the reason this
/// function exists at all.
#[test]
fn bare_option_payload_is_wrapped_like_typed() {
    let doc = schema_of::<WithOptional>().unwrap();
    let store = ObjectStore::default();

    let some =
        serialize_value_with_schema(&value!({ "maybe": 5 }), &checked(&doc), &store).unwrap();
    assert_eq!(some, typed_root(&WithOptional { maybe: Some(5) }));

    let none =
        serialize_value_with_schema(&value!({ "maybe": null }), &checked(&doc), &store).unwrap();
    assert_eq!(none, typed_root(&WithOptional { maybe: None }));
}

/// A `RawTree` node accepts the referenced object id as hex and embeds it as a
/// tree reference, writing no object — matching the typed `RawTree` encoding.
#[test]
fn raw_tree_reference_matches_typed() {
    #[derive(Facet)]
    struct WithRaw {
        raw: RawTree,
    }
    // A pre-written subtree both encodings will reference.
    let (inner, store) = serialize(&Point { x: 1.0, y: 2.0 }).unwrap();
    let typed = serialize_into(
        &WithRaw {
            raw: RawTree::new(inner),
        },
        &store,
    )
    .unwrap();

    let doc = schema_of::<WithRaw>().unwrap();
    let written = serialize_value_with_schema(
        &value!({ "raw": (inner.to_string()) }),
        &checked(&doc),
        &store,
    )
    .unwrap();
    assert_eq!(written, typed);
}

/// A `Node::Dynamic` node delegates to the ordinary dynamic encoder, so it
/// matches a plain `serialize` of the same value.
#[test]
fn dynamic_node_matches_plain_serialize() {
    let doc = schema(Node::Dynamic);
    let v = value!({ "a": [1, 2], "b": "x" });
    let store = ObjectStore::default();
    let written = serialize_value_with_schema(&v, &checked(&doc), &store).unwrap();
    assert_eq!(written, typed_root(&v));
}

// --- conformance failures name the path ---

/// A field the schema names but the object omits entirely (as opposed to
/// setting it `null`) is rejected at write time — the omission would
/// otherwise write a tree `read_struct` can never read back, since it
/// requires an entry for every field, `Optional` included.
#[test]
fn omitting_a_field_is_rejected_at_write() {
    let doc = schema_of::<WithOptional>().unwrap();
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(&value!({}), &checked(&doc), &store).unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::MissingField { field, .. } if field == "maybe"),
        "expected MissingField, got {err:?}"
    );
}

/// An explicit `null` for an `Optional` field, as opposed to omitting the key,
/// still writes and reads back as `None`.
#[test]
fn explicit_null_for_optional_field_round_trips() {
    let doc = schema_of::<WithOptional>().unwrap();
    let store = ObjectStore::default();
    let root = serialize_value_with_schema(&value!({ "maybe": null }), &checked(&doc), &store)
        .expect("an explicit null must write");
    let back =
        deserialize_value_with_schema(&root, &checked(&doc), &store).expect("must read back");
    assert_eq!(back, value!({ "maybe": null }));
}

#[test]
fn unknown_field_is_rejected_with_path() {
    let doc = schema_of::<Person>().unwrap();
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(
        &value!({ "name": "a", "age": 1, "active": true, "extra": 1 }),
        &checked(&doc),
        &store,
    )
    .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::UnknownField { field, .. } if field == "extra"),
        "expected UnknownField, got {err:?}"
    );
}

#[test]
fn out_of_range_integer_is_rejected() {
    // `age: u32`, given a value beyond u32::MAX.
    let doc = schema_of::<Person>().unwrap();
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(
        &value!({ "name": "a", "age": 5000000000i64, "active": true }),
        &checked(&doc),
        &store,
    )
    .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::NumberOutOfRange { schema: "U32", path, .. } if path == "$.age"),
        "expected NumberOutOfRange at $.age, got {err:?}"
    );
}

#[test]
fn float_into_integer_field_is_rejected() {
    let doc = schema_of::<Person>().unwrap();
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(
        &value!({ "name": "a", "age": 1.5, "active": true }),
        &checked(&doc),
        &store,
    )
    .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::Expected { found: "float", path, .. } if path == "$.age"),
        "expected float rejection at $.age, got {err:?}"
    );
}

#[test]
fn integer_too_large_for_f64_is_refused() {
    // 2^53 + 1 has no exact f64 representation.
    let doc = schema(Node::F64);
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(&value!(9007199254740993i64), &checked(&doc), &store)
        .unwrap_err();
    assert!(
        matches!(
            err,
            SchemaWriteError::UnrepresentableNumber { schema: "F64", .. }
        ),
        "expected UnrepresentableNumber, got {err:?}"
    );
}

#[test]
fn type_mismatch_names_the_path() {
    let doc = schema_of::<Person>().unwrap();
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(
        &value!({ "name": "a", "age": 1, "active": "yes" }),
        &checked(&doc),
        &store,
    )
    .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::Expected { expected: "bool", found: "string", path } if path == "$.active"),
        "expected bool/string mismatch at $.active, got {err:?}"
    );
}

#[test]
fn tuple_length_mismatch_is_rejected() {
    // `Event::Move(i32, i32)` — a two-tuple; give it three.
    let doc = schema_of::<Event>().unwrap();
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(&value!({ "Move": [1, 2, 3] }), &checked(&doc), &store)
        .unwrap_err();
    assert!(
        matches!(
            &err,
            SchemaWriteError::LengthMismatch {
                expected: 2,
                found: 3,
                ..
            }
        ),
        "expected LengthMismatch, got {err:?}"
    );
}

#[test]
fn unknown_variant_is_rejected() {
    let doc = schema_of::<Event>().unwrap();
    let store = ObjectStore::default();
    let err =
        serialize_value_with_schema(&value!({ "Nope": null }), &checked(&doc), &store).unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::UnknownVariant { variant, .. } if variant == "Nope"),
        "expected UnknownVariant, got {err:?}"
    );
}

#[test]
fn multi_member_enum_object_is_rejected() {
    let doc = schema_of::<Event>().unwrap();
    let store = ObjectStore::default();
    let mut obj = VObject::new();
    obj.insert("Ping", Value::NULL);
    obj.insert("Message", "hi");
    let err = serialize_value_with_schema(&Value::from(obj), &checked(&doc), &store).unwrap_err();
    assert!(
        matches!(err, SchemaWriteError::MalformedEnum { found: 2, .. }),
        "expected MalformedEnum, got {err:?}"
    );
}

#[test]
fn unknown_ref_is_rejected() {
    // A dangling reference is now rejected when the document is validated,
    // before the write path is entered at all.
    let err = Schema::new("test".into(), Node::Ref("missing".into()), BTreeMap::new()).unwrap_err();
    assert!(
        matches!(&err, SchemaReadError::UnknownRef(name) if name == "missing"),
        "expected UnknownRef, got {err:?}"
    );
}

// --- unit-variant / empty-collection visibility ---

/// A struct with a `priority: Priority` field, `Priority` a plain enum of
/// unit variants (`Low`, `Medium`, `High`). Storing a task and then flipping
/// `priority` from `Low` to `High` must change the `priority` entry's own
/// *blob content* — the property that makes `git diff`/`git log --stat`
/// non-empty and `priority` appear in `git ls-tree -r`.
#[test]
fn priority_field_change_is_a_visible_blob_diff() {
    let priority_variants = BTreeMap::from([
        ("Low".to_owned(), VariantKind::Unit),
        ("Medium".to_owned(), VariantKind::Unit),
        ("High".to_owned(), VariantKind::Unit),
    ]);
    let doc = schema(Node::Struct(one_field(
        "priority",
        Node::Enum(priority_variants),
    )));

    let low_store = ObjectStore::default();
    let low_root = serialize_value_with_schema(
        &value!({ "priority": { "Low": null } }),
        &checked(&doc),
        &low_store,
    )
    .expect("serialize Low");
    let high_store = ObjectStore::default();
    let high_root = serialize_value_with_schema(
        &value!({ "priority": { "High": null } }),
        &checked(&doc),
        &high_store,
    )
    .expect("serialize High");

    assert_ne!(
        low_root, high_root,
        "changing priority must change the task's root id"
    );

    let low_entry = find_entry(&low_store, &low_root, "priority");
    let high_entry = find_entry(&high_store, &high_root, "priority");
    assert_eq!(
        low_entry.mode.kind(),
        EntryKind::Blob,
        "a unit-variant field must be a blob entry, not a tree — this is what makes it \
         visible to `git ls-tree -r`"
    );
    assert_eq!(high_entry.mode.kind(), EntryKind::Blob);
    assert_ne!(
        low_entry.oid, high_entry.oid,
        "the `priority` entry's oid must differ — this is what makes `git diff` non-empty"
    );
    assert_eq!(
        low_store
            .get_blob(&typed(&low_entry.oid))
            .expect("read object")
            .expect("blob"),
        b"Low\n"
    );
    assert_eq!(
        high_store
            .get_blob(&typed(&high_entry.oid))
            .expect("read object")
            .expect("blob"),
        b"High\n"
    );
}

/// An empty collection (here, an empty `tags: List<String>`) writes the
/// presence-marker tree, so `tags` appears as a real (marker) entry in
/// `git ls-tree -r` rather than vanishing as an empty, unlisted directory.
#[test]
fn empty_tags_list_is_visible_as_a_marker_entry() {
    let doc = schema(Node::Struct(one_field(
        "tags",
        Node::List(Box::new(Node::String)),
    )));
    let store = ObjectStore::default();
    let root = serialize_value_with_schema(&value!({ "tags": [] }), &checked(&doc), &store)
        .expect("serialize empty tags");
    let tags_entry = find_entry(&store, &root, "tags");
    assert_eq!(tags_entry.mode.kind(), EntryKind::Tree);
    assert_marker(&store, &tags_entry.oid);

    // And it still reads back as an empty list.
    let v = deserialize_value_with_schema(&root, &checked(&doc), &store).expect("read");
    assert_eq!(v, value!({ "tags": [] }));
}

#[test]
fn unguarded_ref_cycle_is_rejected_before_value_write() {
    // The cycle is rejected when the document is validated, so no value write
    // is ever attempted through it.
    let err = Schema::new(
        "test".into(),
        Node::Ref("loop".into()),
        BTreeMap::from([("loop".into(), Node::Ref("loop".into()))]),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("cycle"),
        "expected an invalid reference cycle, got {err:?}"
    );
}

// --- the marker is distinct from user names and covers every empty shape ---

#[test]
fn an_empty_tuple_uses_the_marker_and_reads_back() {
    let doc = schema(Node::Struct(one_field("nothing", Node::Tuple(vec![]))));
    let store = ObjectStore::default();
    let v = value!({ "nothing": [] });

    let root =
        serialize_value_with_schema(&v, &checked(&doc), &store).expect("write an empty tuple");
    let entry = find_entry(&store, &root, "nothing");
    assert_eq!(entry.mode.kind(), EntryKind::Tree);
    assert_marker(&store, &entry.oid);
    assert_eq!(entry.oid, typed_root(&()));
    let back = deserialize_value_with_schema(&root, &checked(&doc), &store)
        .expect("an empty tuple written by the schema writer must read back");
    assert_eq!(back, v);
}

/// The same, against the typed encoder: a zero-field tuple struct must produce
/// the identical object id through both paths.
#[test]
fn zero_field_tuple_struct_matches_typed() {
    #[derive(Facet, PartialEq, Debug)]
    struct Zero();

    #[derive(Facet, PartialEq, Debug)]
    struct HasZero {
        z: Zero,
    }

    assert_reencodes(HasZero { z: Zero() });
}

#[test]
fn underscore_field_is_distinct_from_the_presence_marker() {
    let doc = schema(Node::Struct(one_field("_", Node::String)));
    let store = ObjectStore::default();
    let input = value!({ "_": "" });
    let root = serialize_value_with_schema(&input, &checked(&doc), &store)
        .expect("write underscore field");
    let entry = find_entry(&store, &root, "_");
    assert_eq!(entry.mode.kind(), EntryKind::Blob);
    assert_eq!(store.get_blob(&typed(&entry.oid)).unwrap().unwrap(), b"\n");
    assert_ne!(root, typed_root(&()));
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        input
    );
}

// --- frozen-format W5 coverage ---

#[test]
fn a_defaulted_field_is_still_required() {
    let doc = schema_of::<WithDefault>().unwrap();
    let Some(Node::Struct(fields)) = doc.defs().get("WithDefault") else {
        panic!("WithDefault must be a named struct definition");
    };
    assert_eq!(fields.get("count"), Some(&Node::U32));
    let store = ObjectStore::default();
    let err =
        serialize_value_with_schema(&value!({ "label": "x" }), &checked(&doc), &store).unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::MissingField { path, field } if path == "$" && field == "count"),
        "expected MissingField for defaulted count, got {err:?}"
    );
    let root = serialize_value_with_schema(
        &value!({ "label": "x", "count": 0 }),
        &checked(&doc),
        &store,
    )
    .unwrap();
    assert_eq!(
        root,
        typed_root(&WithDefault {
            label: "x".into(),
            count: 0,
        })
    );
}

#[test]
fn empty_shapes_share_one_materialized_marker() {
    let cases = [
        (Node::Unit, value!(null)),
        (Node::Optional(Box::new(Node::I32)), value!(null)),
        (Node::Tuple(vec![]), value!([])),
        (Node::Struct(BTreeMap::new()), value!({})),
        (Node::List(Box::new(Node::I32)), value!([])),
        (
            Node::Array {
                elem: Box::new(Node::I32),
                len: 0,
            },
            value!([]),
        ),
        (
            Node::Map {
                key: Box::new(Node::String),
                value: Box::new(Node::I32),
            },
            value!({}),
        ),
    ];
    let expected = typed_root(&());
    for (node, input) in cases {
        let doc = schema(node);
        let store = ObjectStore::default();
        let root =
            serialize_value_with_schema(&input, &checked(&doc), &store).expect("write empty shape");
        assert_eq!(root, expected, "{doc:?}");
        assert_marker(&store, &root);
        assert_eq!(
            deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
            input
        );
    }
}

#[test]
fn schema_writer_escapes_fields_and_map_keys_and_uses_git_order() {
    let doc = schema(Node::Struct(BTreeMap::from([
        ("_".into(), Node::String),
        ("none".into(), Node::String),
        ("some".into(), Node::String),
        (".schema".into(), Node::String),
        ("a/b".into(), Node::String),
        ("naïve".into(), Node::String),
        ("".into(), Node::String),
        ("a.b".into(), Node::String),
        (
            "a".into(),
            Node::Map {
                key: Box::new(Node::String),
                value: Box::new(Node::String),
            },
        ),
    ])));
    let input = value!({
        "_": "underscore", "none": "n", "some": "s", ".schema": "meta",
        "a/b": "slash", "naïve": "unicode", "": "empty", "a.b": "dot",
        "a": { "some": "1", "a/b": "2" }
    });
    let store = ObjectStore::default();
    let root = serialize_value_with_schema(&input, &checked(&doc), &store).unwrap();
    let entries = store.get_tree(&root).unwrap().unwrap();
    let names: Vec<&[u8]> = entries
        .iter()
        .map(|entry| entry.filename.as_ref())
        .collect();
    assert_eq!(
        names,
        [
            "%",
            "%.schema",
            "%a%2Fb",
            "%na%C3%AFve",
            "%none",
            "%some",
            "_",
            "a.b",
            "a"
        ]
        .map(str::as_bytes)
    );
    let map = entries.last().unwrap();
    assert_eq!(map.mode.kind(), EntryKind::Tree);
    let children = store.get_tree(&typed(&map.oid)).unwrap().unwrap();
    let names: Vec<&[u8]> = children
        .iter()
        .map(|entry| entry.filename.as_ref())
        .collect();
    assert_eq!(names, [b"%a%2Fb".as_slice(), b"%some".as_slice()]);
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        input
    );
}

#[test]
fn enum_variants_use_escaped_tags_and_frozen_payload_shapes() {
    let variants = BTreeMap::from([
        ("none".into(), VariantKind::Unit),
        ("a/b".into(), VariantKind::Tuple(vec![Node::String])),
        (
            "some".into(),
            VariantKind::Tuple(vec![Node::I32, Node::Bool]),
        ),
        (
            ".schema".into(),
            VariantKind::Struct(one_field("some", Node::String)),
        ),
    ]);
    let doc = schema(Node::Enum(variants));
    let store = ObjectStore::default();

    let unit = value!({ "none": null });
    let root = serialize_value_with_schema(&unit, &checked(&doc), &store).unwrap();
    assert_eq!(store.get_blob(&root).unwrap().unwrap(), b"%none\n");
    let err = serialize_value_with_schema(&value!({ "none": "payload" }), &checked(&doc), &store)
        .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::Expected { path, expected: "null", .. } if path == "$.none"),
        "unit variant must reject a payload at $.none, got {err:?}"
    );
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        unit
    );

    let direct = value!({ "a/b": "payload" });
    let root = serialize_value_with_schema(&direct, &checked(&doc), &store).unwrap();
    let err = serialize_value_with_schema(&value!({ "a/b": ["payload"] }), &checked(&doc), &store)
        .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::Expected { path, expected: "string", .. } if path == "$.a/b"),
        "one-field tuple payload must be direct, got {err:?}"
    );
    let entry = find_entry(&store, &root, "%a%2Fb");
    assert_eq!(entry.mode.kind(), EntryKind::Blob);
    assert_eq!(
        store.get_blob(&typed(&entry.oid)).unwrap().unwrap(),
        b"payload\n"
    );
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        direct
    );

    let tuple = value!({ "some": [7, true] });
    let root = serialize_value_with_schema(&tuple, &checked(&doc), &store).unwrap();
    let entry = find_entry(&store, &root, "%some");
    assert_eq!(entry.mode.kind(), EntryKind::Tree);
    let children = store.get_tree(&typed(&entry.oid)).unwrap().unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].filename, "000");
    assert_eq!(children[1].filename, "001");
    assert_eq!(
        store.get_blob(&typed(&children[0].oid)).unwrap().unwrap(),
        b"7\n"
    );
    assert_eq!(
        store.get_blob(&typed(&children[1].oid)).unwrap().unwrap(),
        b"true\n"
    );
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        tuple
    );

    let named = value!({ ".schema": { "some": "x" } });
    let root = serialize_value_with_schema(&named, &checked(&doc), &store).unwrap();
    let entry = find_entry(&store, &root, "%.schema");
    assert_eq!(entry.mode.kind(), EntryKind::Tree);
    let child = find_entry(&store, &entry.oid, "%some");
    assert_eq!(store.get_blob(&typed(&child.oid)).unwrap().unwrap(), b"x\n");
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        named
    );
}

#[test]
fn ordinals_are_lowercase_hex_and_enforce_the_tree_cap() {
    let doc = schema(Node::List(Box::new(Node::I32)));
    let mut items = VArray::new();
    for _ in 0..4096 {
        items.push(7i32);
    }
    let input = Value::from(items.clone());
    let store = ObjectStore::default();
    let root =
        serialize_value_with_schema(&input, &checked(&doc), &store).expect("4096 elements fit");
    let entries = store.get_tree(&root).unwrap().unwrap();
    assert_eq!(entries.len(), 4096);
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(
            entry.filename.as_ref() as &[u8],
            format!("{index:03x}").as_bytes()
        );
        assert_eq!(entry.mode.kind(), EntryKind::Blob);
    }
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&doc), &store).unwrap(),
        input
    );
    assert_eq!(root, typed_root(&vec![7i32; 4096]));

    items.push(7i32);
    let err = serialize_value_with_schema(&Value::from(items), &checked(&doc), &store).unwrap_err();
    assert!(matches!(&err, SchemaWriteError::Serialize(_)));
    assert!(err.to_string().contains("4096"), "{err:?}");
}

#[test]
fn float_bridges_use_frozen_text_and_reject_inexact_or_overflowing_numbers() {
    for (node, label, expected) in [
        (Node::F32, "nan", b"nan\n".as_slice()),
        (Node::F32, "inf", b"inf\n".as_slice()),
        (Node::F64, "-inf", b"-inf\n".as_slice()),
    ] {
        let store = ObjectStore::default();
        let root =
            serialize_value_with_schema(&Value::from(label), &checked(&schema(node)), &store)
                .unwrap();
        assert_eq!(store.get_blob(&root).unwrap().unwrap(), expected);
    }
    for (node, typed) in [
        (Node::F32, typed_root(&0.0f32)),
        (Node::F64, typed_root(&0.0f64)),
    ] {
        let store = ObjectStore::default();
        let root =
            serialize_value_with_schema(&Value::from(-0.0f64), &checked(&schema(node)), &store)
                .unwrap();
        assert_eq!(root, typed);
        assert_eq!(store.get_blob(&root).unwrap().unwrap(), b"0.0\n");
    }
    let store = ObjectStore::default();
    let doc = schema(Node::F32);
    let exact = serialize_value_with_schema(&value!(16777216i64), &checked(&doc), &store).unwrap();
    assert_eq!(exact, typed_root(&16777216f32));
    for value in [value!(16777217i64), Value::from(1e40f64)] {
        let err = serialize_value_with_schema(&value, &checked(&doc), &store).unwrap_err();
        assert!(
            matches!(&err, SchemaWriteError::UnrepresentableNumber { path, schema: "F32" } if path == "$"),
            "expected unrepresentable F32, got {err:?}"
        );
    }
    assert!(serialize_value_with_schema(&Value::from("NaN"), &checked(&doc), &store).is_err());
}

#[test]
fn f32_json_max_round_trips_like_typed_f32() {
    // Canonical f32::MAX is `3.4028235e38`: its JSON f64 is slightly above
    // the exact binary maximum but rounds back to the same finite f32.
    let json_max = 3.4028235e38f64;
    assert_eq!(json_max as f32, f32::MAX);
    let store = ObjectStore::default();
    let root =
        serialize_value_with_schema(&Value::from(json_max), &checked(&schema(Node::F32)), &store)
            .expect("canonical f32::MAX JSON must be importable");
    assert_eq!(root, typed_root(&f32::MAX));
}

#[test]
fn byte_and_text_bridges_preserve_their_distinct_leaf_framing() {
    let store = ObjectStore::default();
    for text in ["", "hello\n", "naïve"] {
        let input = Value::from(text);
        let bytes =
            serialize_value_with_schema(&input, &checked(&schema(Node::Bytes)), &store).unwrap();
        let string =
            serialize_value_with_schema(&input, &checked(&schema(Node::String)), &store).unwrap();
        assert_eq!(store.get_blob(&bytes).unwrap().unwrap(), text.as_bytes());
        assert_eq!(
            store.get_blob(&string).unwrap().unwrap(),
            format!("{text}\n").as_bytes()
        );
        assert_eq!(bytes, typed_root(&text.as_bytes().to_vec()));
        assert_ne!(bytes, string);
    }
}

#[test]
fn schema_validation_checks_unused_definitions_and_variants_before_writing() {
    let invalid_map = Node::Map {
        key: Box::new(Node::U8),
        value: Box::new(Node::String),
    };

    // A non-`String` map key in an unused definition is rejected when the
    // document is validated, before any value can be written through it.
    let err = Schema::new(
        "test".into(),
        Node::String,
        BTreeMap::from([("Unused".into(), invalid_map.clone())]),
    )
    .unwrap_err();
    assert!(err.to_string().contains("map key"), "{err:?}");

    // Likewise for a non-`String` map key in an inactive enum variant.
    let err = Schema::new(
        "test".into(),
        Node::Enum(BTreeMap::from([
            ("Ok".into(), VariantKind::Unit),
            ("Bad".into(), VariantKind::Tuple(vec![invalid_map])),
        ])),
        BTreeMap::new(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("map key"), "{err:?}");

    // A dangling reference in an unused definition is reported as such.
    let err = Schema::new(
        "test".into(),
        Node::String,
        BTreeMap::from([("Unused".into(), Node::Ref("missing".into()))]),
    )
    .unwrap_err();
    assert!(
        matches!(&err, SchemaReadError::UnknownRef(name) if name == "missing"),
        "expected dangling ref, got {err:?}"
    );
}

#[test]
fn schema_validation_rejects_oversized_shapes_and_encoded_names() {
    let invalid = [
        Node::Array {
            elem: Box::new(Node::String),
            len: 4097,
        },
        Node::Tuple(vec![Node::Unit; 4097]),
        Node::Struct(one_field(&"/".repeat(85), Node::String)),
        Node::Enum(BTreeMap::from([(
            "Unused".into(),
            VariantKind::Tuple(vec![Node::Unit; 4097]),
        )])),
    ];
    for node in invalid {
        let err = Schema::new(
            "test".into(),
            Node::String,
            BTreeMap::from([("Unused".into(), node)]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid schema"), "{err:?}");
    }
}

#[test]
fn reference_aliases_are_transparent_and_guarded_cycles_are_productive() {
    let aliases = Schema::new(
        "test".into(),
        Node::Ref("A".into()),
        BTreeMap::from([
            ("A".into(), Node::Ref("B".into())),
            ("B".into(), Node::String),
        ]),
    )
    .expect("valid schema");
    let store = ObjectStore::default();
    let input = Value::from("hello");
    let root = serialize_value_with_schema(&input, &checked(&aliases), &store).unwrap();
    assert_eq!(root, typed_root(&String::from("hello")));
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&aliases), &store).unwrap(),
        input
    );

    let recursive = Schema::new(
        "test".into(),
        Node::Ref("Link".into()),
        BTreeMap::from([(
            "Link".into(),
            Node::Struct(BTreeMap::from([
                ("value".into(), Node::I32),
                (
                    "next".into(),
                    Node::Optional(Box::new(Node::Ref("Link".into()))),
                ),
            ])),
        )]),
    )
    .expect("valid schema");
    let input = value!({ "value": 1, "next": { "value": 2, "next": null } });
    let root = serialize_value_with_schema(&input, &checked(&recursive), &store).unwrap();
    assert_eq!(
        deserialize_value_with_schema(&root, &checked(&recursive), &store).unwrap(),
        input
    );
}

#[test]
fn nested_errors_identify_the_value_path() {
    let doc = schema(Node::Struct(one_field(
        "items",
        Node::List(Box::new(Node::U8)),
    )));
    let store = ObjectStore::default();
    let err = serialize_value_with_schema(&value!({ "items": [1, 300] }), &checked(&doc), &store)
        .unwrap_err();
    assert!(
        matches!(&err, SchemaWriteError::NumberOutOfRange { path, schema: "U8", .. } if path == "$.items[1]"),
        "expected range error at $.items[1], got {err:?}"
    );
}
