//! Strict bare-value decoding of foreign Git objects, per `docs/format.adoc`.
//!
//! These fixtures are written independently of the encoder. In particular,
//! raw tree bytes preserve duplicate names, invalid modes, and bad ordering.

#[cfg(feature = "value")]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt::Debug;

use facet::Facet;
use facet_git_tree::{
    DeserializeError, EntryKind, EntryMode, ObjectId, ObjectStore, RawTree, TreeEntry, deserialize,
};
#[cfg(feature = "value")]
use gix_object::{Data, Find};
use gix_object::{Kind, Tree, Write};

#[cfg(feature = "value")]
use facet_git_tree::{Node, Schema, SchemaReadError, deserialize_value_with_schema};

mod common;
use common::Point;

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum Shape {
    Unit,
    Circle { radius: f64 },
    Pair(i32, String),
}

#[derive(Debug, Facet, PartialEq)]
struct Strict {
    a: i32,
    #[facet(default)]
    defaulted: i32,
    optional: Option<String>,
}

#[derive(Debug, Facet, PartialEq)]
struct Empty {}

fn blob(store: &ObjectStore, bytes: &[u8]) -> ObjectId {
    store.write_buf(Kind::Blob, bytes).expect("write blob")
}

/// Entries must already be in Git tree order.
fn tree(store: &ObjectStore, entries: &[(&str, EntryKind, ObjectId)]) -> ObjectId {
    let entries = entries
        .iter()
        .map(|(name, kind, oid)| TreeEntry {
            mode: EntryMode::from(*kind),
            filename: (*name).into(),
            oid: *oid,
        })
        .collect();
    store.write(&Tree { entries }).expect("write tree")
}

/// Bypass tree editors so malformed modes, duplicates, and ordering survive.
fn raw_tree(store: &ObjectStore, entries: &[(&[u8], &[u8], ObjectId)]) -> ObjectId {
    let mut bytes = Vec::new();
    for (mode, name, oid) in entries {
        bytes.extend_from_slice(mode);
        bytes.push(b' ');
        bytes.extend_from_slice(name);
        bytes.push(0);
        bytes.extend_from_slice(oid.as_bytes());
    }
    store.write_buf(Kind::Tree, &bytes).expect("write raw tree")
}

fn marker(store: &ObjectStore) -> ObjectId {
    let empty = blob(store, b"");
    tree(store, &[("none", EntryKind::Blob, empty)])
}

fn rejected<T: for<'a> Facet<'a> + Debug>(store: &ObjectStore, oid: &ObjectId) -> DeserializeError {
    deserialize::<T>(oid, store).expect_err("foreign object must be rejected")
}

fn assert_reflect(error: &DeserializeError, expected: &str) {
    assert!(
        matches!(error, DeserializeError::Reflect(message) if message.contains(expected)),
        "expected error containing {expected:?}, got {error:?}"
    );
}

#[test]
fn missing_object_and_wrong_root_kind_are_distinguished() {
    let store = ObjectStore::default();
    let absent = blob(&store, b"1\n");
    let empty_store = ObjectStore::default();
    let error = rejected::<Point>(&empty_store, &absent);
    assert!(
        matches!(error, DeserializeError::NotFound(id) if id == absent),
        "{error:?}"
    );
    let error = rejected::<Point>(&store, &absent);
    assert!(
        matches!(error, DeserializeError::NotATree(id) if id == absent),
        "{error:?}"
    );
}

#[test]
fn literal_empty_trees_are_rejected_except_for_raw_tree() {
    let store = ObjectStore::default();
    let empty = tree(&store, &[]);
    for error in [
        rejected::<Option<i32>>(&store, &empty),
        rejected::<Vec<i32>>(&store, &empty),
        rejected::<BTreeMap<String, i32>>(&store, &empty),
        rejected::<()>(&store, &empty),
        rejected::<Empty>(&store, &empty),
    ] {
        assert_reflect(&error, "is empty; use the none marker");
    }
    // RawTree is explicitly exempt from the no-empty-tree rule.
    assert_eq!(deserialize::<RawTree>(&empty, &store).unwrap().oid(), empty);
}

#[test]
fn none_and_some_have_distinct_exact_marker_shapes() {
    let store = ObjectStore::default();
    let none = marker(&store);
    let five = blob(&store, b"5\n");
    let some = tree(&store, &[("some", EntryKind::Blob, five)]);
    let some_none = tree(&store, &[("some", EntryKind::Tree, none)]);
    assert_eq!(deserialize::<Option<i32>>(&none, &store).unwrap(), None);
    assert_eq!(deserialize::<Option<i32>>(&some, &store).unwrap(), Some(5));
    assert_eq!(
        deserialize::<Option<Option<i32>>>(&some_none, &store).unwrap(),
        Some(None)
    );
    assert_eq!(
        deserialize::<Vec<i32>>(&none, &store).unwrap(),
        Vec::<i32>::new()
    );
    assert_eq!(deserialize::<Empty>(&none, &store).unwrap(), Empty {});

    let nonempty = blob(&store, b"\n");
    let bad_none = tree(&store, &[("none", EntryKind::Blob, nonempty)]);
    let error = rejected::<Option<i32>>(&store, &bad_none);
    assert_reflect(&error, "is not an empty blob");
    let none_tree = tree(&store, &[("none", EntryKind::Tree, none)]);
    let error = rejected::<Option<i32>>(&store, &none_tree);
    assert_reflect(&error, "none marker must have blob mode");
    let mislabeled = tree(&store, &[("other", EntryKind::Blob, five)]);
    let error = rejected::<Option<i32>>(&store, &mislabeled);
    assert!(
        matches!(&error, DeserializeError::MislabeledOption { name } if name == "other"),
        "{error:?}"
    );
    let two = tree(
        &store,
        &[
            ("none", EntryKind::Blob, blob(&store, b"")),
            ("some", EntryKind::Blob, five),
        ],
    );
    let error = rejected::<Option<i32>>(&store, &two);
    assert!(
        matches!(error, DeserializeError::MalformedOption { found: 2 }),
        "{error:?}"
    );
}

#[test]
fn structural_names_and_noncanonical_escapes_are_rejected() {
    let store = ObjectStore::default();
    let value = blob(&store, b"v\n");
    // A nested map must not interpret reserved document or marker names as keys.
    for name in [".schema", ".value", "some"] {
        let inner = tree(&store, &[(name, EntryKind::Blob, value)]);
        let root = tree(&store, &[("outer", EntryKind::Tree, inner)]);
        let error = rejected::<BTreeMap<String, BTreeMap<String, String>>>(&store, &root);
        assert_reflect(&error, "invalid encoded name");
    }
    for name in ["%a%2fb", "%a%2", "%74itle", "%%FF"] {
        let root = tree(&store, &[(name, EntryKind::Blob, value)]);
        let error = rejected::<BTreeMap<String, String>>(&store, &root);
        assert_reflect(&error, "invalid encoded name");
    }
    let root = raw_tree(&store, &[(b"100644", b"\xff", value)]);
    let error = rejected::<BTreeMap<String, String>>(&store, &root);
    assert!(
        matches!(error, DeserializeError::NonUtf8Name(_)),
        "{error:?}"
    );
    let canonical = tree(&store, &[("%a%2Fb", EntryKind::Blob, value)]);
    assert_eq!(
        deserialize::<BTreeMap<String, String>>(&canonical, &store).unwrap(),
        BTreeMap::from([("a/b".into(), "v".into())])
    );
}

#[test]
fn ordinals_require_three_lowercase_hex_digits_without_gaps_or_duplicates() {
    let store = ObjectStore::default();
    let one = blob(&store, b"1\n");
    for name in ["0", "00", "0000", "00g"] {
        let root = tree(&store, &[(name, EntryKind::Blob, one)]);
        let error = rejected::<Vec<i32>>(&store, &root);
        assert!(
            matches!(&error, DeserializeError::InvalidOrdinal(actual) if actual == name),
            "{name}: {error:?}"
        );
    }
    // The uppercase digit occurs at the correct numeric index, so the only
    // defect is its case. Its lowercase neighbor must decode successfully.
    let mut entries: Vec<_> = (0..10)
        .map(|index| (format!("{index:03x}"), EntryKind::Blob, one))
        .collect();
    entries.push(("00A".into(), EntryKind::Blob, one));
    let refs: Vec<_> = entries
        .iter()
        .map(|(n, k, id)| (n.as_str(), *k, *id))
        .collect();
    let upper = tree(&store, &refs);
    let error = rejected::<Vec<i32>>(&store, &upper);
    assert!(
        matches!(&error, DeserializeError::InvalidOrdinal(name) if name == "00A"),
        "{error:?}"
    );
    entries.last_mut().unwrap().0 = "00a".into();
    let refs: Vec<_> = entries
        .iter()
        .map(|(n, k, id)| (n.as_str(), *k, *id))
        .collect();
    let lower = tree(&store, &refs);
    assert_eq!(
        deserialize::<Vec<i32>>(&lower, &store).unwrap(),
        vec![1; 11]
    );

    let gap = tree(
        &store,
        &[("000", EntryKind::Blob, one), ("002", EntryKind::Blob, one)],
    );
    let error = rejected::<Vec<i32>>(&store, &gap);
    assert_reflect(&error, "ordinal gap: expected 001, found 002");
    let duplicate = raw_tree(
        &store,
        &[(b"100644", b"000", one), (b"100644", b"000", one)],
    );
    let error = rejected::<Vec<i32>>(&store, &duplicate);
    // A raw tree may spell one name twice; that is caught as a duplicate
    // entry name while reading the tree, before ordinal validation runs.
    // `DuplicateOrdinal` is the fallback for distinct spellings of one index,
    // which the canonical fixed-width hex encoding cannot produce.
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("contains duplicate name \"000\"")),
        "two entries named 000 must be rejected as a duplicate name, got {error:?}"
    );
}

#[test]
fn legacy_composite_schema_keyword_is_rejected() {
    // `newtype` was a pre-freeze composite `Node` keyword. Its payload tree
    // decodes structurally but names no current variant, so it is refused.
    let store = ObjectStore::default();
    let inner = blob(&store, b"i32\n");
    let payload = tree(&store, &[("list", EntryKind::Blob, inner)]);
    let root = tree(&store, &[("newtype", EntryKind::Tree, payload)]);
    let error = rejected::<facet_git_tree::Node>(&store, &root);
    assert_reflect(&error, "unknown enum variant \"newtype\"");
}

#[test]
fn out_of_range_integers_are_rejected_on_read() {
    let store = ObjectStore::default();

    // Values too large for a narrower target are refused, not truncated.
    let too_big_for_u8 = blob(&store, b"300\n");
    let error = rejected::<u8>(&store, &too_big_for_u8);
    assert!(
        matches!(&error, DeserializeError::Parse { shape, text, .. } if *shape == "u8" && text == "300"),
        "300 must not read as u8, got {error:?}"
    );

    let below_i8 = blob(&store, b"-129\n");
    let error = rejected::<i8>(&store, &below_i8);
    assert!(
        matches!(&error, DeserializeError::Parse { shape, .. } if *shape == "i8"),
        "-129 must not read as i8, got {error:?}"
    );

    // A signed spelling is non-canonical for an unsigned target.
    let negative = blob(&store, b"-1\n");
    let error = rejected::<u64>(&store, &negative);
    assert!(
        matches!(&error, DeserializeError::Parse { shape, reason, .. } if *shape == "u64" && reason == "non-canonical integer"),
        "-1 must not read as u64, got {error:?}"
    );

    // `isize`/`usize` guard magnitudes their storage cannot hold, before the
    // platform-width narrowing that only a 32-bit target would also exercise.
    let beyond_i64 = blob(&store, b"9223372036854775808\n");
    let error = rejected::<isize>(&store, &beyond_i64);
    assert!(
        matches!(&error, DeserializeError::Parse { shape, reason, .. } if *shape == "isize" && reason == "outside i64 range"),
        "i64::MAX + 1 must not read as isize, got {error:?}"
    );

    let beyond_u64 = blob(&store, b"18446744073709551616\n");
    let error = rejected::<usize>(&store, &beyond_u64);
    assert!(
        matches!(&error, DeserializeError::Parse { shape, reason, .. } if *shape == "usize" && reason == "outside u64 range"),
        "u64::MAX + 1 must not read as usize, got {error:?}"
    );
}

#[test]
fn tuple_variant_ordinal_gap_is_not_a_missing_named_field() {
    let store = ObjectStore::default();
    let one = blob(&store, b"1\n");
    let text = blob(&store, b"two\n");
    let payload = tree(
        &store,
        &[
            ("000", EntryKind::Blob, one),
            ("002", EntryKind::Blob, text),
        ],
    );
    let root = tree(&store, &[("Pair", EntryKind::Tree, payload)]);
    let error = rejected::<Shape>(&store, &root);
    assert_reflect(&error, "ordinal gap: expected 001, found 002");
}

#[test]
fn tree_entry_cap_and_fixed_array_length_are_enforced() {
    let store = ObjectStore::default();
    let one = blob(&store, b"1\n");
    let names: Vec<_> = (0..=4096).map(|index| format!("{index:03x}")).collect();
    let entries: Vec<_> = names
        .iter()
        .map(|name| (name.as_str(), EntryKind::Blob, one))
        .collect();
    let at_cap = tree(&store, &entries[..4096]);
    assert_eq!(
        deserialize::<Vec<i32>>(&at_cap, &store).unwrap().len(),
        4096
    );
    // The extra four-digit name sorts before the three-digit names beginning
    // with 2..f. Keep the raw tree Git-ordered so only the cap is at issue.
    let mut over_entries = entries.clone();
    over_entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let over_cap = tree(&store, &over_entries);
    let error = rejected::<Vec<i32>>(&store, &over_cap);
    assert_reflect(&error, "exceeds the maximum of 4096 entries");
    let short = tree(&store, &[("000", EntryKind::Blob, one)]);
    let error = rejected::<[i32; 2]>(&store, &short);
    assert_reflect(&error, "sequence length mismatch: expected 2, found 1");
    let long = tree(
        &store,
        &[
            ("000", EntryKind::Blob, one),
            ("001", EntryKind::Blob, one),
            ("002", EntryKind::Blob, one),
        ],
    );
    let error = rejected::<[i32; 2]>(&store, &long);
    assert_reflect(&error, "sequence length mismatch: expected 2, found 3");
}

#[test]
fn entry_modes_and_referenced_object_kinds_are_checked() {
    let store = ObjectStore::default();
    let one = blob(&store, b"1\n");
    let invalid_mode = raw_tree(&store, &[(b"100755", b"000", one)]);
    let error = rejected::<Vec<i32>>(&store, &invalid_mode);
    assert_reflect(&error, "invalid or non-canonical entry mode");
    let blob_as_tree = tree(&store, &[("some", EntryKind::Tree, one)]);
    let error = rejected::<Option<i32>>(&store, &blob_as_tree);
    assert!(
        matches!(error, DeserializeError::NotATree(id) if id == one),
        "{error:?}"
    );
    let subtree = marker(&store);
    let tree_as_blob = tree(&store, &[("some", EntryKind::Blob, subtree)]);
    let error = rejected::<Option<i32>>(&store, &tree_as_blob);
    assert!(
        matches!(error, DeserializeError::NotABlob(id) if id == subtree),
        "{error:?}"
    );
}

#[test]
fn enum_tags_have_one_entry_and_the_expected_payload_shape() {
    let store = ObjectStore::default();
    let value = blob(&store, b"1\n");
    let payload = marker(&store);
    let multiple = tree(
        &store,
        &[
            ("Circle", EntryKind::Tree, payload),
            ("Pair", EntryKind::Tree, payload),
        ],
    );
    let error = rejected::<Shape>(&store, &multiple);
    assert!(
        matches!(error, DeserializeError::MalformedEnum { found: 2 }),
        "{error:?}"
    );
    let unit_as_tree = tree(&store, &[("Unit", EntryKind::Tree, payload)]);
    let error = rejected::<Shape>(&store, &unit_as_tree);
    assert!(
        matches!(&error, DeserializeError::UnitVariantIsTree { variant } if variant == "Unit"),
        "{error:?}"
    );
    let pair_as_blob = blob(&store, b"Pair\n");
    let error = rejected::<Shape>(&store, &pair_as_blob);
    assert!(
        matches!(&error, DeserializeError::VariantPayloadIsBlob { variant } if variant == "Pair"),
        "{error:?}"
    );
    let unknown = tree(&store, &[("Unknown", EntryKind::Blob, value)]);
    let error = rejected::<Shape>(&store, &unknown);
    assert_reflect(&error, "unknown enum variant \"Unknown\"");
    let bad_keyword = blob(&store, b"not_a_schema_node\n");
    let error = rejected::<facet_git_tree::Node>(&store, &bad_keyword);
    assert_reflect(&error, "unknown enum variant \"not_a_schema_node\"");
}

#[test]
fn every_struct_field_is_required_even_if_optional_or_defaulted() {
    let store = ObjectStore::default();
    let a = blob(&store, b"7\n");
    let defaulted = blob(&store, b"0\n");
    let none = marker(&store);
    let valid = tree(
        &store,
        &[
            ("a", EntryKind::Blob, a),
            ("defaulted", EntryKind::Blob, defaulted),
            ("optional", EntryKind::Tree, none),
        ],
    );
    assert_eq!(
        deserialize::<Strict>(&valid, &store).unwrap(),
        Strict {
            a: 7,
            defaulted: 0,
            optional: None
        }
    );
    let fixtures = [
        (
            tree(
                &store,
                &[
                    ("defaulted", EntryKind::Blob, defaulted),
                    ("optional", EntryKind::Tree, none),
                ],
            ),
            "a",
        ),
        (
            tree(
                &store,
                &[
                    ("a", EntryKind::Blob, a),
                    ("optional", EntryKind::Tree, none),
                ],
            ),
            "defaulted",
        ),
        (
            tree(
                &store,
                &[
                    ("a", EntryKind::Blob, a),
                    ("defaulted", EntryKind::Blob, defaulted),
                ],
            ),
            "optional",
        ),
    ];
    for (root, missing) in fixtures {
        let error = rejected::<Strict>(&store, &root);
        assert!(
            matches!(&error, DeserializeError::MissingField { field } if field == missing),
            "{missing}: {error:?}"
        );
    }
    let unknown = tree(
        &store,
        &[
            ("a", EntryKind::Blob, a),
            ("defaulted", EntryKind::Blob, defaulted),
            ("optional", EntryKind::Tree, none),
            ("z", EntryKind::Blob, a),
        ],
    );
    let error = rejected::<Strict>(&store, &unknown);
    assert!(
        matches!(&error, DeserializeError::UnexpectedEntry { entry } if entry == "z"),
        "{error:?}"
    );
    let radius = blob(&store, b"1.0\n");
    let extra = tree(
        &store,
        &[
            ("extra", EntryKind::Blob, a),
            ("radius", EntryKind::Blob, radius),
        ],
    );
    let root = tree(&store, &[("Circle", EntryKind::Tree, extra)]);
    let error = rejected::<Shape>(&store, &root);
    assert!(
        matches!(&error, DeserializeError::UnexpectedEntry { entry } if entry == "extra"),
        "{error:?}"
    );
}

#[test]
fn noncanonical_scalar_text_and_missing_newline_are_rejected() {
    let store = ObjectStore::default();
    let missing_newline = blob(&store, b"Unit");
    let error = rejected::<Shape>(&store, &missing_newline);
    assert!(
        matches!(error, DeserializeError::MissingLeafNewline(id) if id == missing_newline),
        "{error:?}"
    );
    for text in ["+1", "01", "-0"] {
        let root = blob(&store, format!("{text}\n").as_bytes());
        let error = rejected::<i32>(&store, &root);
        assert!(
            matches!(&error, DeserializeError::Parse { text: actual, .. } if actual == text),
            "{error:?}"
        );
    }
    for text in ["-0.0", "1.00", "NaN"] {
        let root = blob(&store, format!("{text}\n").as_bytes());
        let error = rejected::<f64>(&store, &root);
        assert!(
            matches!(&error, DeserializeError::Parse { text: actual, .. } if actual == text),
            "{error:?}"
        );
    }
    let boolean = blob(&store, b"True\n");
    let error = rejected::<bool>(&store, &boolean);
    assert!(
        matches!(&error, DeserializeError::Parse { text, .. } if text == "True"),
        "{error:?}"
    );
    let character = blob(&store, b"ab\n");
    let error = rejected::<char>(&store, &character);
    assert!(
        matches!(&error, DeserializeError::Parse { text, .. } if text == "ab"),
        "{error:?}"
    );
}

#[test]
fn unsorted_and_duplicate_trees_are_rejected() {
    let store = ObjectStore::default();
    let value = blob(&store, b"1\n");
    let unsorted = raw_tree(
        &store,
        &[(b"100644", b"b", value), (b"100644", b"a", value)],
    );
    let error = rejected::<BTreeMap<String, i32>>(&store, &unsorted);
    assert_reflect(&error, "not in Git entry order");
    let duplicate = raw_tree(
        &store,
        &[(b"100644", b"a", value), (b"100644", b"a", value)],
    );
    let error = rejected::<BTreeMap<String, i32>>(&store, &duplicate);
    assert_reflect(&error, "contains duplicate name \"a\"");
}

#[cfg(feature = "value")]
fn schema(root: Node, defs: BTreeMap<String, Node>) -> Schema {
    Schema::new("test".into(), root, defs).expect("valid schema")
}

#[cfg(feature = "value")]
#[test]
fn invalid_refs_cycles_and_non_string_map_keys_are_rejected() {
    let error =
        Schema::new("test".into(), Node::Ref("missing".into()), BTreeMap::new()).unwrap_err();
    assert!(
        matches!(&error, SchemaReadError::UnknownRef(name) if name == "missing"),
        "{error:?}"
    );
    let error = Schema::new(
        "test".into(),
        Node::Ref("A".into()),
        BTreeMap::from([("A".into(), Node::Ref("A".into()))]),
    )
    .unwrap_err();
    assert!(
        matches!(&error, SchemaReadError::Deserialize(DeserializeError::Reflect(message)) if message.contains("unguarded reference cycle")),
        "{error:?}"
    );
    let error = Schema::new(
        "test".into(),
        Node::Map {
            key: Box::new(Node::I32),
            value: Box::new(Node::String),
        },
        BTreeMap::new(),
    )
    .unwrap_err();
    assert!(
        matches!(&error, SchemaReadError::Deserialize(DeserializeError::Reflect(message)) if message.contains("map key must be literally String")),
        "{error:?}"
    );
}

#[cfg(feature = "value")]
#[test]
fn cbor_must_be_canonical_and_contain_exactly_one_item() {
    let store = ObjectStore::default();
    let doc = schema(Node::Cbor, BTreeMap::new());
    let doc = doc.checked().expect("valid schema");
    let canonical = blob(&store, &[0]);
    assert!(deserialize_value_with_schema(&canonical, &doc, &store).is_ok());
    for bytes in [&[0x18, 0x00][..], &[0x00, 0x00][..]] {
        let root = blob(&store, bytes);
        let error = deserialize_value_with_schema(&root, &doc, &store).unwrap_err();
        assert!(
            matches!(&error, SchemaReadError::Deserialize(DeserializeError::Reflect(message)) if message.contains("invalid CBOR blob")),
            "{bytes:02x?}: {error:?}"
        );
    }
}

#[test]
fn typed_recursive_value_enforces_encoded_depth_boundary() {
    #[derive(Debug, Facet)]
    struct DeepNode {
        children: Vec<DeepNode>,
    }

    fn nested_nodes(store: &ObjectStore, wraps: usize) -> ObjectId {
        let empty_list = marker(store);
        let mut node = tree(store, &[("children", EntryKind::Tree, empty_list)]);
        for _ in 0..wraps {
            let list = tree(store, &[("000", EntryKind::Tree, node)]);
            node = tree(store, &[("children", EntryKind::Tree, list)]);
        }
        node
    }

    let store = ObjectStore::default();
    // At 15 wraps the terminal marker blob is at encoded depth 32.
    let at_limit = nested_nodes(&store, 15);
    let mut current: DeepNode = deserialize(&at_limit, &store).expect("depth 32 must be allowed");
    for _ in 0..15 {
        assert_eq!(current.children.len(), 1);
        current = current.children.pop().unwrap();
    }
    assert!(current.children.is_empty());
    let too_deep = nested_nodes(&store, 16);
    let error = rejected::<DeepNode>(&store, &too_deep);
    assert!(matches!(error, DeserializeError::MaxDepth(32)), "{error:?}");
}

#[cfg(feature = "value")]
fn nested_maps(store: &ObjectStore, levels: usize) -> ObjectId {
    let mut current = blob(store, b"leaf\n");
    let mut kind = EntryKind::Blob;
    for _ in 0..levels {
        current = tree(store, &[("x", kind, current)]);
        kind = EntryKind::Tree;
    }
    current
}

#[cfg(feature = "value")]
#[test]
fn encoded_depth_32_is_allowed_and_depth_33_is_rejected() {
    let store = ObjectStore::default();
    let doc = schema(Node::Dynamic, BTreeMap::new());
    let doc = doc.checked().expect("valid schema");
    let at_limit = nested_maps(&store, 32);
    assert!(
        deserialize_value_with_schema(&at_limit, &doc, &store).is_ok(),
        "depth 32 must be allowed"
    );
    let too_deep = nested_maps(&store, 33);
    let error = deserialize_value_with_schema(&too_deep, &doc, &store).unwrap_err();
    assert!(
        matches!(
            error,
            SchemaReadError::Deserialize(DeserializeError::MaxDepth(32))
        ),
        "{error:?}"
    );
}

#[cfg(feature = "value")]
struct CountingStore<'a> {
    store: &'a ObjectStore,
    finds: Cell<usize>,
}

#[cfg(feature = "value")]
impl Find for CountingStore<'_> {
    fn try_find<'a>(
        &self,
        id: &gix_hash::oid,
        buffer: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
        self.finds.set(self.finds.get() + 1);
        self.store.try_find(id, buffer)
    }
}

#[cfg(feature = "value")]
fn sequence_of_shared(
    store: &ObjectStore,
    child: ObjectId,
    kind: EntryKind,
    count: usize,
) -> ObjectId {
    let entries = (0..count)
        .map(|index| TreeEntry {
            mode: EntryMode::from(kind),
            filename: format!("{index:03x}").into(),
            oid: child,
        })
        .collect();
    store
        .write(&Tree { entries })
        .expect("write shared sequence")
}

#[cfg(feature = "value")]
#[test]
fn visit_budget_counts_each_occurrence_of_shared_oids() {
    const LIMIT: usize = 1_048_576;
    let store = ObjectStore::default();
    let leaf = blob(&store, b"1\n");
    let full = sequence_of_shared(&store, leaf, EntryKind::Blob, 4095);
    let short = sequence_of_shared(&store, leaf, EntryKind::Blob, 4094);
    let mut branches = (0..256)
        .map(|i| (format!("{i:03x}"), EntryKind::Tree, full))
        .collect::<Vec<_>>();
    branches.last_mut().unwrap().2 = short;
    let refs: Vec<_> = branches
        .iter()
        .map(|(n, k, id)| (n.as_str(), *k, *id))
        .collect();
    let at_limit = tree(&store, &refs);
    branches.last_mut().unwrap().2 = full;
    let refs: Vec<_> = branches
        .iter()
        .map(|(n, k, id)| (n.as_str(), *k, *id))
        .collect();
    let over_limit = tree(&store, &refs);
    let doc = schema(
        Node::List(Box::new(Node::List(Box::new(Node::I32)))),
        BTreeMap::new(),
    );
    let doc = doc.checked().expect("valid schema");
    let counted = CountingStore {
        store: &store,
        finds: Cell::new(0),
    };

    // 1 root + 255 * (1 tree + 4095 leaves) + (1 tree + 4094 leaves).
    let result = deserialize_value_with_schema(&at_limit, &doc, &counted);
    assert!(
        result.is_ok(),
        "exactly {LIMIT} visits must succeed: {result:?}"
    );
    drop(result);
    assert_eq!(counted.finds.get(), LIMIT);
    counted.finds.set(0);
    // One more visit, but only three distinct reachable oids in this graph.
    let error = deserialize_value_with_schema(&over_limit, &doc, &counted).unwrap_err();
    assert!(
        matches!(&error, SchemaReadError::Deserialize(DeserializeError::Reflect(message)) if message.contains("maximum decode visits (1048576) exceeded")),
        "{error:?}"
    );
    assert_eq!(
        counted.finds.get(),
        LIMIT,
        "the over-budget object must not be fetched"
    );

    counted.finds.set(0);
    let error = deserialize::<Vec<Vec<i32>>>(&over_limit, &counted)
        .expect_err("typed reads must share the visit limit");
    assert_reflect(&error, "maximum decode visits (1048576) exceeded");
    assert_eq!(counted.finds.get(), LIMIT);
}
