//! Integration tests for schema generation (`schema_of`).
//!
//! Covers spec requirements:
//!   schema.generation
//!     — the shape → schema converter mirrors the encoder's dispatch order
//!       (transparency collapse, RawTree, dynamic values, scalars, byte
//!       sequences, composites), deduplicates named user types into `defs`
//!       with deterministic `_2`-suffixed names, and breaks recursion with
//!       `Node::Ref`.
//!   schema.representation
//!     — the `Node`/`Schema` shapes asserted here are the public,
//!       semver-major on-disk contract.

use std::collections::{BTreeMap, HashMap};

use facet::Facet;
use facet_git_tree::{
    Node, RawTree, Schema, SchemaError, VariantKind, deserialize, schema_of, serialize,
};

mod common;
use common::{
    Event, Nested, Person, Point, TreeNode, WithArray, WithDefault, WithMap, WithOptional,
    WithValue, WithVec,
};

// --- helpers ---

fn fields(pairs: Vec<(&str, Node)>) -> BTreeMap<String, Node> {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

fn variants(pairs: Vec<(&str, VariantKind)>) -> BTreeMap<String, VariantKind> {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

fn re(name: &str) -> Node {
    Node::Ref(name.into())
}

fn doc(kind: &str, root: Node, defs: Vec<(&str, Node)>) -> Schema {
    Schema::new(
        kind.into(),
        root,
        defs.into_iter().map(|(k, v)| (k.into(), v)).collect(),
    )
    .expect("valid schema")
}

fn point_def() -> Node {
    Node::Struct(fields(vec![("x", Node::F64), ("y", Node::F64)]))
}

// --- named structs enter defs, referenced by Ref ---

/// A named struct becomes a `Ref` root with its body in `defs`.
#[test]
fn point_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<Point>()?,
        doc("Point", re("Point"), vec![("Point", point_def())])
    );
    Ok(())
}

/// Scalar fields map through the per-width scalar table, in declaration order.
#[test]
fn person_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<Person>()?,
        doc(
            "Person",
            re("Person"),
            vec![(
                "Person",
                Node::Struct(fields(vec![
                    ("name", Node::String),
                    ("age", Node::U32),
                    ("active", Node::Bool),
                ]))
            )]
        )
    );
    Ok(())
}

/// Defaulted fields have the same required-field schema as all other fields.
#[test]
fn defaulted_field_has_no_special_schema_marker() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<WithDefault>()?,
        doc(
            "WithDefault",
            re("WithDefault"),
            vec![(
                "WithDefault",
                Node::Struct(fields(vec![("label", Node::String), ("count", Node::U32)]))
            )]
        )
    );
    Ok(())
}

/// A struct-valued field is itself deduplicated into `defs`.
#[test]
fn nested_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<Nested>()?,
        doc(
            "Nested",
            re("Nested"),
            vec![
                (
                    "Nested",
                    Node::Struct(fields(vec![
                        ("location", re("Point")),
                        ("label", Node::String),
                    ]))
                ),
                ("Point", point_def()),
            ]
        )
    );
    Ok(())
}

// --- collections ---

/// A `Vec<T>` field is a `List` node.
#[test]
fn with_vec_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<WithVec>()?,
        doc(
            "WithVec",
            re("WithVec"),
            vec![(
                "WithVec",
                Node::Struct(fields(vec![("items", Node::List(Box::new(Node::I64)))]))
            )]
        )
    );
    Ok(())
}

/// A `[T; N]` field is an `Array` node carrying its exact length.
#[test]
fn with_array_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<WithArray>()?,
        doc(
            "WithArray",
            re("WithArray"),
            vec![(
                "WithArray",
                Node::Struct(fields(vec![(
                    "values",
                    Node::Array {
                        elem: Box::new(Node::I32),
                        len: 4
                    }
                )]))
            )]
        )
    );
    Ok(())
}

/// A map field carries both key and value schemas.
#[test]
fn with_map_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<WithMap>()?,
        doc(
            "WithMap",
            re("WithMap"),
            vec![(
                "WithMap",
                Node::Struct(fields(vec![(
                    "table",
                    Node::Map {
                        key: Box::new(Node::String),
                        value: Box::new(Node::String)
                    }
                )]))
            )]
        )
    );
    Ok(())
}

/// An `Option<T>` field is an `Optional` node.
#[test]
fn with_optional_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<WithOptional>()?,
        doc(
            "WithOptional",
            re("WithOptional"),
            vec![(
                "WithOptional",
                Node::Struct(fields(vec![("maybe", Node::Optional(Box::new(Node::I32)))]))
            )]
        )
    );
    Ok(())
}

/// A `u8` sequence is `Bytes`, not a `List` — mirroring the blob encoding.
#[test]
fn byte_seq_schema_is_bytes() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<Vec<u8>>()?,
        Schema::new("Vec".into(), Node::Bytes, BTreeMap::new()).expect("valid schema")
    );
    Ok(())
}

/// Only string keys have a native map schema; other key types are CBOR.
#[test]
fn non_string_key_maps_have_opaque_cbor_schema() -> anyhow::Result<()> {
    assert_eq!(schema_of::<HashMap<i32, String>>()?.root(), &Node::Cbor);
    assert_eq!(
        schema_of::<BTreeMap<(u8, u8), String>>()?.root(),
        &Node::Cbor
    );
    Ok(())
}

#[test]
fn array_length_is_bounded_by_the_tree_entry_cap() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<[i32; 4096]>()?.root(),
        &Node::Array {
            elem: Box::new(Node::I32),
            len: 4096,
        }
    );
    assert!(
        matches!(
            schema_of::<[i32; 4097]>(),
            Err(SchemaError::UnsupportedShape(_))
        ),
        "an array schema cannot declare more than 4096 entries"
    );
    Ok(())
}

// --- unit ---

/// Both `()` and a unit-valued field use the unit marker schema.
#[test]
fn unit_field_schema_is_unit() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct WithUnit {
        u: (),
    }
    assert_eq!(schema_of::<()>()?, doc("()", Node::Unit, vec![]));
    assert_eq!(
        schema_of::<WithUnit>()?,
        doc(
            "WithUnit",
            re("WithUnit"),
            vec![("WithUnit", Node::Struct(fields(vec![("u", Node::Unit)])))]
        )
    );
    Ok(())
}

/// A unit struct is a named definition whose body is `Node::Unit`.
#[test]
fn unit_struct_schema_is_unit() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct Marker;
    assert_eq!(
        schema_of::<Marker>()?,
        doc("Marker", re("Marker"), vec![("Marker", Node::Unit)])
    );
    Ok(())
}

// --- enums ---

/// Unit, tuple and struct variants retain their distinct payload kinds;
/// a single-field tuple has no separate schema variant.
#[test]
fn enum_schema() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<Event>()?,
        doc(
            "Event",
            re("Event"),
            vec![(
                "Event",
                Node::Enum(variants(vec![
                    ("Ping", VariantKind::Unit),
                    ("Message", VariantKind::Tuple(vec![Node::String])),
                    ("Move", VariantKind::Tuple(vec![Node::I32, Node::I32])),
                    (
                        "Login",
                        VariantKind::Struct(fields(vec![
                            ("user", Node::String),
                            ("ok", Node::Bool),
                        ]))
                    ),
                ]))
            )]
        )
    );
    Ok(())
}

/// Effective names remain user names in the schema; escaping happens on disk.
#[test]
fn schema_uses_effective_names_without_pre_escaping() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct Renamed {
        #[facet(rename = "none")]
        presence: String,
        #[facet(rename = ".schema")]
        schema: u32,
        task: RenamedTask,
    }

    #[derive(Facet)]
    #[repr(u8)]
    #[facet(rename_all = "snake_case")]
    enum RenamedTask {
        #[facet(rename = "some")]
        Wait,
        LongTask(String),
        Named {
            #[facet(rename = "a/b")]
            label: String,
        },
    }

    let tasks = [
        RenamedTask::Wait,
        RenamedTask::LongTask("payload".into()),
        RenamedTask::Named { label: "x".into() },
    ];
    for task in tasks {
        let (root, store) = serialize(&task)?;
        match task {
            RenamedTask::Wait => {
                assert_eq!(store.get_blob(&root)?.as_deref(), Some(&b"%some\n"[..]));
            }
            RenamedTask::LongTask(payload) => {
                let tag = common::find_entry(&store, &root, "long_task");
                assert_eq!(
                    store.get_blob(&tag.oid)?,
                    Some(format!("{payload}\n").into_bytes())
                );
            }
            RenamedTask::Named { label } => {
                let tag = common::find_entry(&store, &root, "named");
                let field = common::find_entry(&store, &tag.oid, "%a%2Fb");
                assert_eq!(
                    store.get_blob(&field.oid)?,
                    Some(format!("{label}\n").into_bytes())
                );
            }
        }
    }
    let schema = schema_of::<Renamed>()?;
    assert_eq!(schema.kind(), "Renamed");
    let Node::Ref(root_name) = schema.root() else {
        panic!("named struct must have a reference root");
    };
    let Some(Node::Struct(root_fields)) = schema.defs().get(root_name) else {
        panic!("root must resolve to struct fields");
    };
    assert_eq!(
        root_fields,
        &fields(vec![
            ("none", Node::String),
            (".schema", Node::U32),
            ("task", re("RenamedTask")),
        ])
    );
    assert_eq!(
        schema.defs().get("RenamedTask"),
        Some(&Node::Enum(variants(vec![
            ("some", VariantKind::Unit),
            ("long_task", VariantKind::Tuple(vec![Node::String])),
            (
                "named",
                VariantKind::Struct(fields(vec![("a/b", Node::String)])),
            ),
        ])))
    );
    Ok(())
}

/// A complete bare schema uses ordinary frozen `Node`/`VariantKind` trees.
#[test]
fn frozen_schema_wire_golden_and_roundtrip() -> anyhow::Result<()> {
    let schema = doc(
        "Demo",
        re("Root"),
        vec![(
            "Root",
            Node::Struct(fields(vec![
                ("count", Node::U32),
                (
                    "event",
                    Node::Enum(variants(vec![
                        ("Ping", VariantKind::Unit),
                        ("Message", VariantKind::Tuple(vec![Node::String])),
                    ])),
                ),
            ])),
        )],
    );
    let (root, store) = serialize(&schema)?;
    // Independently hashed Git objects for the exact frozen layout below.
    assert_eq!(root.to_string(), "12660b786043b54b79da3c0f6ea135e16f9a92d0");
    let root_entries = common::tree_entries(&store, &root);
    let names: Vec<&[u8]> = root_entries
        .iter()
        .map(|entry| entry.filename.as_ref())
        .collect();
    assert_eq!(names, [b"defs".as_slice(), b"kind", b"root"]);
    let kind = common::find_entry(&store, &root, "kind");
    assert_eq!(store.get_blob(&kind.oid)?.as_deref(), Some(&b"Demo\n"[..]));
    let defs = common::find_entry(&store, &root, "defs");
    let root_def = common::find_entry(&store, &defs.oid, "Root");
    let structure = common::find_entry(&store, &root_def.oid, "struct");
    let count = common::find_entry(&store, &structure.oid, "count");
    assert_eq!(store.get_blob(&count.oid)?.as_deref(), Some(&b"u32\n"[..]));
    let event = common::find_entry(&store, &structure.oid, "event");
    let enum_payload = common::find_entry(&store, &event.oid, "enum");
    let message = common::find_entry(&store, &enum_payload.oid, "Message");
    let tuple = common::find_entry(&store, &message.oid, "tuple");
    let first = common::find_entry(&store, &tuple.oid, "000");
    assert_eq!(
        store.get_blob(&first.oid)?.as_deref(),
        Some(&b"string\n"[..])
    );
    let decoded: Schema = deserialize(&root, &store)?;
    assert_eq!(decoded, schema);
    assert_eq!(serialize(&decoded)?.0, root);
    Ok(())
}

// --- special leaves ---

/// A `RawTree` field is the `RawTree` schema node.
#[test]
fn raw_tree_field_schema() -> anyhow::Result<()> {
    #[derive(Facet)]
    struct WithRaw {
        raw: RawTree,
    }
    assert_eq!(
        schema_of::<WithRaw>()?,
        doc(
            "WithRaw",
            re("WithRaw"),
            vec![(
                "WithRaw",
                Node::Struct(fields(vec![("raw", Node::RawTree)]))
            )]
        )
    );
    Ok(())
}

/// A `facet_value::Value` field is the `Dynamic` schema node.
#[test]
fn value_field_schema_is_dynamic() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<WithValue>()?,
        doc(
            "WithValue",
            re("WithValue"),
            vec![(
                "WithValue",
                Node::Struct(fields(vec![("meta", Node::Dynamic)]))
            )]
        )
    );
    Ok(())
}

// --- recursion ---

/// A self-referential type resolves its own occurrences to `Ref`, breaking
/// the cycle through `defs`.
#[test]
fn recursive_schema_uses_ref() -> anyhow::Result<()> {
    assert_eq!(
        schema_of::<TreeNode>()?,
        doc(
            "TreeNode",
            re("TreeNode"),
            vec![(
                "TreeNode",
                Node::Struct(fields(vec![
                    ("value", Node::I64),
                    ("children", Node::List(Box::new(re("TreeNode")))),
                ]))
            )]
        )
    );
    Ok(())
}

// --- name collisions ---

/// Distinct types sharing an identifier get deterministic `_2` suffixes in
/// pre-order.
#[test]
fn name_collision_disambiguated() -> anyhow::Result<()> {
    mod a {
        #[derive(facet::Facet)]
        pub struct Dup {
            pub x: i32,
        }
    }
    mod b {
        #[derive(facet::Facet)]
        pub struct Dup {
            pub y: u8,
        }
    }
    #[derive(Facet)]
    struct Both {
        first: a::Dup,
        second: b::Dup,
    }
    assert_eq!(
        schema_of::<Both>()?,
        doc(
            "Both",
            re("Both"),
            vec![
                (
                    "Both",
                    Node::Struct(fields(vec![("first", re("Dup")), ("second", re("Dup_2"))]))
                ),
                ("Dup", Node::Struct(fields(vec![("x", Node::I32)]))),
                ("Dup_2", Node::Struct(fields(vec![("y", Node::U8)]))),
            ]
        )
    );
    Ok(())
}

// --- transparency ---

/// A smart pointer collapses to its pointee's schema, but keeps its own kind.
#[test]
fn box_schema_collapses() -> anyhow::Result<()> {
    let boxed = schema_of::<Box<Point>>()?;
    let point = schema_of::<Point>()?;
    assert_eq!(boxed.kind(), "Box");
    assert_eq!(point.kind(), "Point");
    assert_eq!(boxed.root(), point.root());
    assert_eq!(boxed.defs(), point.defs());
    Ok(())
}

/// A transparent newtype collapses to its inner type's schema.
#[test]
fn transparent_newtype_schema_collapses() -> anyhow::Result<()> {
    #[derive(Facet)]
    #[facet(transparent)]
    struct Hex(String);
    assert_eq!(
        schema_of::<Hex>()?,
        Schema::new("Hex".into(), Node::String, BTreeMap::new()).expect("valid schema")
    );
    Ok(())
}

// --- depth guard ---

/// A shape nested deeper than the walker's bound is refused with
/// `SchemaError::MaxDepth` rather than recursing unboundedly — data written
/// that deep could never be read back by the depth-bounded deserializer
/// regardless of what schema described it.
///
/// Exercised through `from_shape_with_limit` with a small bound: a type
/// actually deeper than the codec depth bound (32) makes the compiler's recursive
/// `SHAPE` evaluation prohibitively expensive, and the guard's threading is
/// identical at every bound.
#[test]
fn excessively_nested_shape_schema_is_rejected() {
    type Nested = Option<Option<Option<Option<Option<i32>>>>>;
    let err = Schema::from_shape_with_limit(<Nested as facet::Facet>::SHAPE, 3).unwrap_err();
    assert!(
        matches!(err, SchemaError::MaxDepth(3)),
        "expected MaxDepth(3), got {err:?}"
    );
    // The same shape is comfortably within the real bound: the default-bound
    // entry point produces the full optional chain rather than failing.
    let mut expected = Node::I32;
    for _ in 0..5 {
        expected = Node::Optional(Box::new(expected));
    }
    assert_eq!(schema_of::<Nested>().unwrap().root(), &expected);
}

/// The explicit-bound entry point at the default bound reproduces
/// `from_shape` exactly — same document.
#[test]
fn from_shape_with_limit_at_default_matches_from_shape() -> anyhow::Result<()> {
    let via_limit = Schema::from_shape_with_limit(<common::Person as facet::Facet>::SHAPE, 32)?;
    let via_default = Schema::from_shape(<common::Person as facet::Facet>::SHAPE)?;
    assert_eq!(via_limit, via_default);
    Ok(())
}
