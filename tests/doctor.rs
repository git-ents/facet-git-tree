//! Verification of the schema-of-schemas frozen object ids and fixed point.

use std::collections::BTreeMap;

use facet_git_tree::doctor;

pub use facet_git_tree::{
    DeserializeError, ObjectId, ObjectStore, Schema, SchemaError, SerializeError, deserialize,
    schema_of, serialize_into,
};
use facet_git_tree::{Node, VariantKind};
use gix_hash::Kind as HashKind;

fn expected_schema() -> Schema {
    let r = |name: &str| Node::Ref(name.to_owned());
    let string_map = |value| Node::Map {
        key: Box::new(Node::String),
        value: Box::new(value),
    };
    let fields = |entries: Vec<(&str, Node)>| {
        entries
            .into_iter()
            .map(|(name, node)| (name.to_owned(), node))
            .collect()
    };
    let mut node_variants = BTreeMap::new();
    for name in [
        "unit", "bool", "char", "string", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
        "u32", "u64", "u128", "usize", "f32", "f64", "bytes", "cbor", "rawtree", "dynamic",
    ] {
        node_variants.insert(name.to_owned(), VariantKind::Unit);
    }
    node_variants.insert("list".into(), VariantKind::Tuple(vec![r("Node")]));
    node_variants.insert(
        "array".into(),
        VariantKind::Struct(fields(vec![("elem", r("Node")), ("len", Node::USize)])),
    );
    node_variants.insert(
        "map".into(),
        VariantKind::Struct(fields(vec![("key", r("Node")), ("value", r("Node"))])),
    );
    node_variants.insert("optional".into(), VariantKind::Tuple(vec![r("Node")]));
    node_variants.insert(
        "tuple".into(),
        VariantKind::Tuple(vec![Node::List(Box::new(r("Node")))]),
    );
    node_variants.insert(
        "struct".into(),
        VariantKind::Tuple(vec![string_map(r("Node"))]),
    );
    node_variants.insert(
        "enum".into(),
        VariantKind::Tuple(vec![string_map(r("VariantKind"))]),
    );
    node_variants.insert("ref".into(), VariantKind::Tuple(vec![Node::String]));
    let variants = BTreeMap::from([
        ("unit".into(), VariantKind::Unit),
        (
            "tuple".into(),
            VariantKind::Tuple(vec![Node::List(Box::new(r("Node")))]),
        ),
        (
            "struct".into(),
            VariantKind::Tuple(vec![string_map(r("Node"))]),
        ),
    ]);
    Schema {
        kind: "Schema".into(),
        root: r("Schema"),
        defs: BTreeMap::from([
            (
                "Schema".into(),
                Node::Struct(fields(vec![
                    ("kind", Node::String),
                    ("root", r("Node")),
                    ("defs", string_map(r("Node"))),
                ])),
            ),
            ("Node".into(), Node::Enum(node_variants)),
            ("VariantKind".into(), Node::Enum(variants)),
        ]),
    }
}

#[test]
fn s0_matches_frozen_schema_and_ids_in_both_hash_formats() {
    let schema = schema_of::<Schema>().expect("derive S0");
    assert_eq!(schema, expected_schema());
    let cases = [
        (
            HashKind::Sha1,
            doctor::S0_SHA1,
            "b8b61390b23a8ddcd72ab6171c676e89fbf67fad",
        ),
        (
            HashKind::Sha256,
            doctor::S0_SHA256,
            "e490630b57486d68a19d90bcd9a9def27ba9f58760cccf06e7eb083039fdc5e8",
        ),
    ];
    let report = doctor::check().expect("both S0 checks");
    assert_eq!(report.sha1, doctor::S0_SHA1);
    assert_eq!(report.sha256, doctor::S0_SHA256);

    for (hash_kind, expected, expected_hex) in cases {
        assert_eq!(expected.kind(), hash_kind);
        assert_eq!(expected.to_string(), expected_hex);
        assert_eq!(
            doctor::check_against(hash_kind, expected).expect("S0 check"),
            expected
        );

        let store = ObjectStore::with_hash_kind(hash_kind);
        assert_eq!(
            serialize_into(&schema, &store).expect("encode S0"),
            expected
        );
        let decoded: Schema = deserialize(&expected, &store).expect("decode S0");
        assert_eq!(decoded, schema);
        assert_eq!(
            serialize_into(&decoded, &store).expect("re-encode S0"),
            expected
        );
    }
}

#[test]
fn check_rejects_wrong_pins_in_both_hash_formats() {
    for (hash_kind, actual) in [
        (HashKind::Sha1, doctor::S0_SHA1),
        (HashKind::Sha256, doctor::S0_SHA256),
    ] {
        let wrong = ObjectId::empty_tree(hash_kind);
        let error = doctor::check_against(hash_kind, wrong).expect_err("wrong pin must fail");
        assert!(matches!(
            error,
            doctor::DoctorError::DigestMismatch {
                hash_kind: found_kind,
                expected,
                actual: found,
            } if found_kind == hash_kind && expected == wrong && found == actual
        ));
    }
}

#[test]
fn check_rejects_pins_from_the_wrong_hash_format() {
    for (hash_kind, wrong) in [
        (HashKind::Sha1, doctor::S0_SHA256),
        (HashKind::Sha256, doctor::S0_SHA1),
    ] {
        let error = doctor::check_against(hash_kind, wrong).expect_err("cross-format pin");
        assert!(matches!(
            error,
            doctor::DoctorError::HashFormat {
                hash_kind: found_kind,
                expected,
            } if found_kind == hash_kind && expected == wrong.kind()
        ));
    }
}
