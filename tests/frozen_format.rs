//! Independent fixtures for the bare codec described by `docs/format.adoc`.
//!
//! `serialize` and `deserialize` operate on bare values, not the documented
//! `.schema`/`.value` document envelope. The foreign-tree helper writes raw
//! Git tree object bytes so it does not repair malformed modes or ordering.

use std::collections::BTreeMap;
use std::fmt::Debug;

use facet::Facet;
use facet_git_tree::{DeserializeError, Node, ObjectId, ObjectStore, deserialize, serialize};
use gix_object::{Kind, Write as _};
use proptest::prelude::*;

const BLOB: u16 = 0o100644;
const TREE: u16 = 0o040000;

#[derive(Clone, Debug, Facet, PartialEq)]
struct Book {
    #[facet(rename = ".schema")]
    metadata: String,
    #[facet(rename = "a")]
    annotations: BTreeMap<String, String>,
    #[facet(rename = "a.b")]
    title: String,
    #[facet(rename = "a/b")]
    subtitle: String,
    #[facet(rename = "some")]
    comment: Option<String>,
    bytes: Vec<u8>,
    count: i32,
    task: Task,
}

#[derive(Clone, Debug, Facet, PartialEq)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
enum Task {
    #[facet(rename = "none")]
    Waiting,
    UnitCamel,
    #[facet(rename = "a/b")]
    Note(String),
    Wrapped(Vec<i32>),
    Pair(i32, bool),
    Named {
        #[facet(rename = ".value")]
        label: String,
    },
}

#[derive(Debug, Facet, PartialEq)]
struct Empty {}

#[derive(Debug, Facet, PartialEq)]
struct Strict {
    name: String,
    #[facet(default)]
    number: i32,
    maybe: Option<i32>,
}

#[derive(Debug, Facet, PartialEq)]
struct One {
    x: i32,
}

#[derive(Debug, Facet, PartialEq)]
struct OneOption {
    x: Option<i32>,
}

#[derive(Debug, Facet, PartialEq)]
struct GitOrder {
    a: Vec<i32>,
    #[facet(rename = "a.b")]
    dot: String,
}

#[derive(Debug, Facet, PartialEq)]
struct Seventeen(
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
);

#[derive(Debug, Facet, PartialEq)]
struct Generated {
    text: String,
    bytes: Vec<u8>,
    numbers: Vec<i16>,
    tags: BTreeMap<String, String>,
    maybe: Option<i32>,
    task: Task,
}

fn blob(store: &ObjectStore, bytes: &[u8]) -> ObjectId {
    store.write_buf(Kind::Blob, bytes).expect("write blob")
}

// Raw entries are (ASCII mode, filename bytes, object id), already in the
// intended on-disk order. In particular, duplicate names are not deduplicated.
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
    store.write_buf(Kind::Tree, &bytes).expect("write raw tree")
}

fn assert_tree(store: &ObjectStore, oid: &ObjectId, expected: &[(&str, u16)]) -> Vec<ObjectId> {
    let entries = store.get_tree(oid).expect("read object").expect("tree");
    assert_eq!(entries.len(), expected.len(), "tree {oid}: {entries:?}");
    for (entry, (name, mode)) in entries.iter().zip(expected) {
        let actual: &[u8] = entry.filename.as_ref();
        assert_eq!(actual, name.as_bytes(), "tree {oid}");
        assert_eq!(entry.mode.value(), *mode, "entry {name:?} in tree {oid}");
    }
    entries.into_iter().map(|entry| entry.oid).collect()
}

fn assert_blob(store: &ObjectStore, oid: &ObjectId, expected: &[u8]) {
    assert_eq!(
        store.get_blob(oid).expect("read object").expect("blob"),
        expected,
        "blob {oid}"
    );
}

fn assert_fixed_point<T>(store: &ObjectStore, oid: &ObjectId, expected: &T)
where
    T: for<'a> Facet<'a> + Debug + PartialEq,
{
    let decoded: T = deserialize(oid, store).expect("decode canonical value");
    assert_eq!(&decoded, expected);
    let (encoded, _) = serialize(&decoded).expect("re-encode canonical value");
    assert_eq!(&encoded, oid, "encode(decode(tree)) must preserve its oid");
}

/// Decode a deliberately malformed fixture, requiring a *structural*
/// rejection. `NotFound`, a backend error, and `Unsupported` (the target type
/// has no codec at all) would each make the fixture vacuous, so they are
/// refused here rather than silently accepted as "it errored". Returns the
/// error so call sites can pin the specific rule that fired.
fn assert_rejected<T>(store: &ObjectStore, oid: &ObjectId, case: &str) -> DeserializeError
where
    T: for<'a> Facet<'a>,
{
    let error: DeserializeError = deserialize::<T>(oid, store)
        .err()
        .unwrap_or_else(|| panic!("accepted malformed {case}"));
    assert!(
        !matches!(
            error,
            DeserializeError::Unsupported(_)
                | DeserializeError::NotFound(_)
                | DeserializeError::Backend(_)
        ),
        "rejected {case} for the wrong reason (not a structural rule): {error:?}"
    );
    error
}

fn assert_rejected_leaf<T>(store: &ObjectStore, bytes: &[u8]) -> DeserializeError
where
    T: for<'a> Facet<'a>,
{
    let oid = blob(store, bytes);
    assert_rejected::<T>(store, &oid, &format!("leaf {bytes:?}"))
}

#[test]
fn book_golden_is_a_bare_git_tree_with_ordered_fields_and_exact_blobs() {
    let book = Book {
        metadata: "manifest\n".into(),
        annotations: BTreeMap::from([("a".into(), "one".into()), ("a.b".into(), "two".into())]),
        title: "A Tale".into(),
        subtitle: String::new(),
        comment: Some("note".into()),
        bytes: vec![0, 0xff, b'\n'],
        count: -42,
        task: Task::Pair(-12, true),
    };
    let (root, store) = serialize(&book).expect("encode book");
    // SHA-1 of the independently specified raw Git tree and its child objects.
    assert_eq!(root.to_string(), "8f554d146dee7a53a1d636bf114563bc3885b93d");
    let fields = assert_tree(
        &store,
        &root,
        &[
            ("%.schema", BLOB),
            ("%a%2Fb", BLOB),
            ("%some", TREE),
            ("a.b", BLOB),
            ("a", TREE),
            ("bytes", BLOB),
            ("count", BLOB),
            ("task", TREE),
        ],
    );
    assert_blob(&store, &fields[0], b"manifest\n\n");
    assert_blob(&store, &fields[1], b"\n");
    let some = assert_tree(&store, &fields[2], &[("some", BLOB)]);
    assert_blob(&store, &some[0], b"note\n");
    assert_blob(&store, &fields[3], b"A Tale\n");
    let annotations = assert_tree(&store, &fields[4], &[("a", BLOB), ("a.b", BLOB)]);
    assert_blob(&store, &annotations[0], b"one\n");
    assert_blob(&store, &annotations[1], b"two\n");
    assert_blob(&store, &fields[5], b"\0\xff\n");
    assert_blob(&store, &fields[6], b"-42\n");
    let task = assert_tree(&store, &fields[7], &[("pair", TREE)]);
    let pair = assert_tree(&store, &task[0], &[("000", BLOB), ("001", BLOB)]);
    assert_blob(&store, &pair[0], b"-12\n");
    assert_blob(&store, &pair[1], b"true\n");
    assert_fixed_point(&store, &root, &book);
}

#[test]
fn bare_roots_are_not_document_envelopes() {
    let (number, number_store) = serialize(&42i32).expect("scalar");
    assert_blob(&number_store, &number, b"42\n");
    assert_fixed_point(&number_store, &number, &42i32);

    let (unit_variant, variant_store) = serialize(&Task::UnitCamel).expect("unit variant");
    assert_blob(&variant_store, &unit_variant, b"unit_camel\n");
    assert_fixed_point(&variant_store, &unit_variant, &Task::UnitCamel);

    let (record, record_store) = serialize(&One { x: 42 }).expect("struct");
    let entries = assert_tree(&record_store, &record, &[("x", BLOB)]);
    assert_blob(&record_store, &entries[0], b"42\n");
    assert_fixed_point(&record_store, &record, &One { x: 42 });

    let (option, option_store) = serialize(&Some(42i32)).expect("option");
    let entries = assert_tree(&option_store, &option, &[("some", BLOB)]);
    assert_blob(&option_store, &entries[0], b"42\n");
    assert_fixed_point(&option_store, &option, &Some(42i32));
}

#[test]
fn names_escape_canonically_and_maps_use_encoded_git_order() {
    let values: BTreeMap<String, String> = [
        "title",
        "author.name",
        "_",
        "none",
        "some",
        ".schema",
        ".value",
        ".git",
        "a/b",
        "a b",
        "naïve",
        "",
        "%",
        "\0",
    ]
    .into_iter()
    .map(|key| (key.to_owned(), "v".to_owned()))
    .collect();
    let (root, store) = serialize(&values).expect("encode map");
    let entries = assert_tree(
        &store,
        &root,
        &[
            ("%", BLOB),
            ("%%00", BLOB),
            ("%%25", BLOB),
            ("%.git", BLOB),
            ("%.schema", BLOB),
            ("%.value", BLOB),
            ("%a%20b", BLOB),
            ("%a%2Fb", BLOB),
            ("%na%C3%AFve", BLOB),
            ("%none", BLOB),
            ("%some", BLOB),
            ("_", BLOB),
            ("author.name", BLOB),
            ("title", BLOB),
        ],
    );
    for oid in entries {
        assert_blob(&store, &oid, b"v\n");
    }
    assert_fixed_point(&store, &root, &values);
}

#[test]
fn name_limit_counts_the_encoded_bytes() {
    for key in ["a".repeat(255), "/".repeat(84), "é".repeat(42)] {
        let map = BTreeMap::from([(key, 1i32)]);
        let (root, store) = serialize(&map).expect("name within encoded limit");
        assert_fixed_point(&store, &root, &map);
    }
    for key in ["a".repeat(256), "/".repeat(85), "é".repeat(43)] {
        let map = BTreeMap::from([(key, 1i32)]);
        assert!(serialize(&map).is_err(), "encoded name exceeds 255 bytes");
    }
}

#[test]
fn task_variants_use_effective_names_and_payload_shapes() {
    let waiting = Task::Waiting;
    let (root, store) = serialize(&waiting).expect("unit");
    assert_blob(&store, &root, b"%none\n");
    assert_fixed_point(&store, &root, &waiting);

    let note = Task::Note("payload\n".into());
    let (root, store) = serialize(&note).expect("one-field tuple variant");
    assert_eq!(root.to_string(), "5b024ad909a6d573aa3626994866318c03bf2f0f");
    let tag = assert_tree(&store, &root, &[("%a%2Fb", BLOB)]);
    assert_blob(&store, &tag[0], b"payload\n\n");
    assert_fixed_point(&store, &root, &note);

    let wrapped = Task::Wrapped(vec![11, 12]);
    let (root, store) = serialize(&wrapped).expect("direct tree payload");
    let tag = assert_tree(&store, &root, &[("wrapped", TREE)]);
    let list = assert_tree(&store, &tag[0], &[("000", BLOB), ("001", BLOB)]);
    assert_blob(&store, &list[0], b"11\n");
    assert_blob(&store, &list[1], b"12\n");
    assert_fixed_point(&store, &root, &wrapped);

    let pair = Task::Pair(-3, false);
    let (root, store) = serialize(&pair).expect("tuple variant");
    let tag = assert_tree(&store, &root, &[("pair", TREE)]);
    let fields = assert_tree(&store, &tag[0], &[("000", BLOB), ("001", BLOB)]);
    assert_blob(&store, &fields[0], b"-3\n");
    assert_blob(&store, &fields[1], b"false\n");
    assert_fixed_point(&store, &root, &pair);

    let named = Task::Named { label: "x".into() };
    let (root, store) = serialize(&named).expect("named variant");
    let tag = assert_tree(&store, &root, &[("named", TREE)]);
    let fields = assert_tree(&store, &tag[0], &[("%.value", BLOB)]);
    assert_blob(&store, &fields[0], b"x\n");
    assert_fixed_point(&store, &root, &named);
}

#[test]
fn meta_schema_nodes_use_lowercase_tags_and_ordinary_payloads() {
    let leaf = Node::String;
    let (root, store) = serialize(&leaf).expect("schema leaf");
    assert_blob(&store, &root, b"string\n");
    assert_fixed_point(&store, &root, &leaf);

    let list = Node::List(Box::new(Node::String));
    let (root, store) = serialize(&list).expect("schema list");
    let tag = assert_tree(&store, &root, &[("list", BLOB)]);
    assert_blob(&store, &tag[0], b"string\n");
    assert_fixed_point(&store, &root, &list);

    let array = Node::Array {
        elem: Box::new(Node::I32),
        len: 2,
    };
    let (root, store) = serialize(&array).expect("schema array");
    let tag = assert_tree(&store, &root, &[("array", TREE)]);
    let fields = assert_tree(&store, &tag[0], &[("elem", BLOB), ("len", BLOB)]);
    assert_blob(&store, &fields[0], b"i32\n");
    assert_blob(&store, &fields[1], b"2\n");
    assert_fixed_point(&store, &root, &array);

    let reference = Node::Ref("Book".into());
    let (root, store) = serialize(&reference).expect("schema ref");
    let tag = assert_tree(&store, &root, &[("ref", BLOB)]);
    assert_blob(&store, &tag[0], b"Book\n");
    assert_fixed_point(&store, &root, &reference);
}

#[test]
fn unit_empty_collections_and_options_use_marker_trees() {
    let (unit, unit_store) = serialize(&()).expect("unit");
    let marker = assert_tree(&unit_store, &unit, &[("none", BLOB)]);
    assert_blob(&unit_store, &marker[0], b"");
    assert_fixed_point(&unit_store, &unit, &());

    let (empty_struct, empty_store) = serialize(&Empty {}).expect("empty struct");
    assert_eq!(empty_struct, unit);
    assert_fixed_point(&empty_store, &empty_struct, &Empty {});

    let (empty_list, list_store) = serialize(&Vec::<i32>::new()).expect("empty list");
    assert_eq!(empty_list, unit);
    assert_fixed_point(&list_store, &empty_list, &Vec::<i32>::new());

    let empty_map = BTreeMap::<String, String>::new();
    let (map, map_store) = serialize(&empty_map).expect("empty map");
    assert_eq!(map, unit);
    assert_fixed_point(&map_store, &map, &empty_map);

    let (none, none_store) = serialize(&None::<i32>).expect("none");
    assert_eq!(none, unit);
    assert_fixed_point(&none_store, &none, &None::<i32>);

    let some_none = Some(None::<i32>);
    let (some, some_store) = serialize(&some_none).expect("some none");
    let entries = assert_tree(&some_store, &some, &[("some", TREE)]);
    assert_eq!(entries[0], unit);
    assert_fixed_point(&some_store, &some, &some_none);

    let some_unit = Some(());
    let (some, some_store) = serialize(&some_unit).expect("some unit");
    let entries = assert_tree(&some_store, &some, &[("some", TREE)]);
    assert_eq!(entries[0], unit);
    assert_fixed_point(&some_store, &some, &some_unit);
}

#[test]
fn text_and_binary_blobs_are_distinct_including_when_empty() {
    let text = String::from("hello\n");
    let (text_oid, text_store) = serialize(&text).expect("text");
    assert_blob(&text_store, &text_oid, b"hello\n\n");
    assert_fixed_point(&text_store, &text_oid, &text);

    let binary = b"hello\n".to_vec();
    let (binary_oid, binary_store) = serialize(&binary).expect("bytes");
    assert_blob(&binary_store, &binary_oid, b"hello\n");
    assert_ne!(binary_oid, text_oid);
    assert_fixed_point(&binary_store, &binary_oid, &binary);

    let (empty_text, text_store) = serialize(&String::new()).expect("empty text");
    assert_blob(&text_store, &empty_text, b"\n");
    let (empty_bytes, bytes_store) = serialize(&Vec::<u8>::new()).expect("empty bytes");
    assert_blob(&bytes_store, &empty_bytes, b"");
    assert_ne!(empty_text, empty_bytes);
    assert_fixed_point(&bytes_store, &empty_bytes, &Vec::<u8>::new());

    let array = [0, 255, b'\n'];
    let (array_oid, array_store) = serialize(&array).expect("byte array");
    assert_blob(&array_store, &array_oid, b"\0\xff\n");
    assert_fixed_point(&array_store, &array_oid, &array);
    let too_short = blob(&array_store, b"\0\xff");
    assert_rejected::<[u8; 3]>(&array_store, &too_short, "short byte array");
    let too_long = blob(&array_store, b"\0\xff\nX");
    assert_rejected::<[u8; 3]>(&array_store, &too_long, "long byte array");
}

#[test]
fn list_and_tuple_ordinals_cross_hexadecimal_boundaries() {
    let list = vec![7i16; 4096];
    let (root, store) = serialize(&list).expect("4096 elements fit");
    let entries = store.get_tree(&root).expect("read list").expect("tree");
    assert_eq!(entries.len(), 4096);
    for (index, entry) in entries.iter().enumerate() {
        let actual: &[u8] = entry.filename.as_ref();
        assert_eq!(actual, format!("{index:03x}").as_bytes());
        assert_eq!(entry.mode.value(), BLOB);
    }
    for index in [0, 9, 10, 15, 16, 255, 256, 4095] {
        assert_blob(&store, &entries[index].oid, b"7\n");
    }
    assert!(serialize(&vec![7i16; 4097]).is_err(), "entry cap");

    let tuple = Seventeen(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16);
    let (root, store) = serialize(&tuple).expect("tuple struct");
    let entries = store.get_tree(&root).expect("read tuple").expect("tree");
    assert_eq!(entries.len(), 17);
    for (index, entry) in entries.iter().enumerate() {
        let actual: &[u8] = entry.filename.as_ref();
        assert_eq!(actual, format!("{index:03x}").as_bytes());
        assert_eq!(entry.mode.value(), BLOB);
        assert_blob(&store, &entry.oid, format!("{index}\n").as_bytes());
    }
    assert_fixed_point(&store, &root, &tuple);
}

#[test]
fn git_directory_comparator_is_not_plain_string_order() {
    let value = GitOrder {
        a: vec![1],
        dot: "dot".into(),
    };
    let (root, store) = serialize(&value).expect("encode");
    let entries = assert_tree(&store, &root, &[("a.b", BLOB), ("a", TREE)]);
    assert_blob(&store, &entries[0], b"dot\n");
    let child = assert_tree(&store, &entries[1], &[("000", BLOB)]);
    assert_blob(&store, &child[0], b"1\n");
    assert_fixed_point(&store, &root, &value);

    let backwards = foreign_tree(
        &store,
        &[
            (b"40000", b"a", entries[1]),
            (b"100644", b"a.b", entries[0]),
        ],
    );
    assert_rejected::<GitOrder>(&store, &backwards, "non-Git tree order");
}

#[test]
fn foreign_noncanonical_or_structural_map_names_are_rejected() {
    let store = ObjectStore::default();
    let value = blob(&store, b"v\n");
    let names: &[&[u8]] = &[
        b"%74itle",
        b"%%74itle",
        b"%title",
        b"%a%2fb",
        b"%%2f",
        b"%a%2",
        b"%%GG",
        b"%%FF",
        b"a b",
        b"a%2Fb",
        b".schema",
        b".value",
        b"none",
        b"some",
        b"\xff",
    ];
    for name in names {
        let root = foreign_tree(&store, &[(b"100644", name, value)]);
        assert_rejected::<BTreeMap<String, String>>(&store, &root, &format!("name {name:?}"));
    }
    let long = vec![b'x'; 256];
    let root = foreign_tree(&store, &[(b"100644", &long, value)]);
    assert_rejected::<BTreeMap<String, String>>(&store, &root, "256-byte name");
}

#[test]
fn forbidden_nested_structural_names_and_duplicate_aliases_are_rejected() {
    let store = ObjectStore::default();
    let text = blob(&store, b"value\n");
    for name in [b".schema".as_slice(), b".value", b"none", b"some"] {
        let map = foreign_tree(&store, &[(b"100644", name, text)]);
        let root = foreign_tree(&store, &[(b"40000", b"a", map)]);
        assert_rejected::<BTreeMap<String, BTreeMap<String, String>>>(
            &store,
            &root,
            &format!("nested structural name {name:?}"),
        );
    }
    // Both names decode to the same logical key, but only the verbatim form
    // is canonical. A map reader must not silently merge them.
    let aliased = foreign_tree(
        &store,
        &[(b"100644", b"%%61", text), (b"100644", b"a", text)],
    );
    assert_rejected::<BTreeMap<String, String>>(&store, &aliased, "escaped duplicate key");
}

#[test]
fn foreign_ordinals_and_array_lengths_are_rejected() {
    let store = ObjectStore::default();
    let one = blob(&store, b"1\n");
    for name in [b"0".as_slice(), b"00", b"0000", b"001", b"00g"] {
        let root = foreign_tree(&store, &[(b"100644", name, one)]);
        assert_rejected::<Vec<i32>>(&store, &root, &format!("ordinal {name:?}"));
    }
    let mut upper: Vec<ForeignEntry<'_>> = [
        b"000".as_slice(),
        b"001",
        b"002",
        b"003",
        b"004",
        b"005",
        b"006",
        b"007",
        b"008",
        b"009",
        b"00A",
    ]
    .into_iter()
    .map(|name| (b"100644".as_slice(), name, one))
    .collect();
    let root = foreign_tree(&store, &upper);
    let error = assert_rejected::<Vec<i32>>(&store, &root, "uppercase hexadecimal ordinal");
    assert!(
        matches!(error, DeserializeError::InvalidOrdinal(ref name) if name == "00A"),
        "uppercase digit must be an invalid ordinal, got {error:?}"
    );
    upper.pop();
    upper.push((b"100644", b"00a", one));
    let root = foreign_tree(&store, &upper);
    let expected = vec![1i32; 11];
    assert_fixed_point(&store, &root, &expected);

    let gap = foreign_tree(
        &store,
        &[(b"100644", b"000", one), (b"100644", b"002", one)],
    );
    let error = assert_rejected::<Vec<i32>>(&store, &gap, "ordinal gap");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("ordinal gap: expected 001, found 002")),
        "{error:?}"
    );
    let duplicate = foreign_tree(
        &store,
        &[(b"100644", b"000", one), (b"100644", b"000", one)],
    );
    let error = assert_rejected::<Vec<i32>>(&store, &duplicate, "duplicate ordinal");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("contains duplicate name \"000\"")),
        "{error:?}"
    );
    let unsorted = foreign_tree(
        &store,
        &[(b"100644", b"001", one), (b"100644", b"000", one)],
    );
    let error = assert_rejected::<Vec<i32>>(&store, &unsorted, "unsorted ordinals");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("is not in Git entry order")),
        "out-of-order entries must fail the Git-order check first: {error:?}"
    );
    let short = foreign_tree(&store, &[(b"100644", b"000", one)]);
    assert_rejected::<[i32; 2]>(&store, &short, "short array");
    let long = foreign_tree(
        &store,
        &[
            (b"100644", b"000", one),
            (b"100644", b"001", one),
            (b"100644", b"002", one),
        ],
    );
    assert_rejected::<[i32; 2]>(&store, &long, "long array");
    let empty = foreign_tree(&store, &[]);
    assert_rejected::<Vec<i32>>(&store, &empty, "literal empty list tree");

    let names: Vec<_> = (0..=4096).map(|index| format!("{index:03x}")).collect();
    let over_cap: Vec<_> = names
        .iter()
        .map(|name| (b"100644".as_slice(), name.as_bytes(), one))
        .collect();
    let root = foreign_tree(&store, &over_cap);
    assert_rejected::<Vec<i32>>(&store, &root, "4097 sequence entries");
}

#[test]
fn missing_even_default_or_optional_fields_and_unknown_fields_are_rejected() {
    let store = ObjectStore::default();
    let name = blob(&store, b"n\n");
    let number = blob(&store, b"9\n");
    let empty_blob = blob(&store, b"");
    let none = foreign_tree(&store, &[(b"100644", b"none", empty_blob)]);
    let valid = foreign_tree(
        &store,
        &[
            (b"40000", b"maybe", none),
            (b"100644", b"name", name),
            (b"100644", b"number", number),
        ],
    );
    assert_fixed_point(
        &store,
        &valid,
        &Strict {
            name: "n".into(),
            number: 9,
            maybe: None,
        },
    );

    let no_name = foreign_tree(
        &store,
        &[(b"40000", b"maybe", none), (b"100644", b"number", number)],
    );
    let error = assert_rejected::<Strict>(&store, &no_name, "missing required field");
    assert!(
        matches!(error, DeserializeError::MissingField { ref field } if field == "name"),
        "{error:?}"
    );
    let no_default = foreign_tree(
        &store,
        &[(b"40000", b"maybe", none), (b"100644", b"name", name)],
    );
    let error = assert_rejected::<Strict>(&store, &no_default, "missing defaulted field");
    assert!(
        matches!(error, DeserializeError::MissingField { ref field } if field == "number"),
        "every field is required, defaults included: {error:?}"
    );
    let no_option = foreign_tree(
        &store,
        &[(b"100644", b"name", name), (b"100644", b"number", number)],
    );
    let error = assert_rejected::<Strict>(&store, &no_option, "missing optional field");
    assert!(
        matches!(error, DeserializeError::MissingField { ref field } if field == "maybe"),
        "an `Option` field is the `none` marker, never omitted: {error:?}"
    );
    let unknown = foreign_tree(
        &store,
        &[
            (b"40000", b"maybe", none),
            (b"100644", b"name", name),
            (b"100644", b"number", number),
            (b"100644", b"z", name),
        ],
    );
    let error = assert_rejected::<Strict>(&store, &unknown, "unknown field");
    assert!(
        matches!(error, DeserializeError::UnexpectedEntry { ref entry } if entry == "z"),
        "{error:?}"
    );
    let duplicate = foreign_tree(
        &store,
        &[(b"100644", b"x", number), (b"100644", b"x", number)],
    );
    let error = assert_rejected::<One>(&store, &duplicate, "duplicate field");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("contains duplicate name \"x\"")),
        "{error:?}"
    );
}

#[test]
fn foreign_modes_kinds_order_and_duplicate_map_entries_are_rejected() {
    let store = ObjectStore::default();
    let one = blob(&store, b"1\n");
    for mode in [
        b"100600".as_slice(),
        b"100664",
        b"100755",
        b"120000",
        b"160000",
    ] {
        let root = foreign_tree(&store, &[(mode, b"x", one)]);
        assert_rejected::<One>(&store, &root, &format!("mode {mode:?}"));
    }
    let wrong_kind = foreign_tree(&store, &[(b"40000", b"x", one)]);
    assert_rejected::<One>(&store, &wrong_kind, "tree mode pointing to blob");
    let empty = blob(&store, b"");
    let none = foreign_tree(&store, &[(b"100644", b"none", empty)]);
    let wrong_kind = foreign_tree(&store, &[(b"100644", b"x", none)]);
    assert_rejected::<OneOption>(&store, &wrong_kind, "blob mode pointing to tree");
    let out_of_order = foreign_tree(&store, &[(b"100644", b"b", one), (b"100644", b"a", one)]);
    assert_rejected::<BTreeMap<String, i32>>(&store, &out_of_order, "unsorted map");
    let duplicate = foreign_tree(&store, &[(b"100644", b"a", one), (b"100644", b"a", one)]);
    assert_rejected::<BTreeMap<String, i32>>(&store, &duplicate, "duplicate map key");
}

#[test]
fn malformed_markers_and_options_are_rejected() {
    let store = ObjectStore::default();
    let empty_blob = blob(&store, b"");
    let nonempty_blob = blob(&store, b"\n");
    let value = blob(&store, b"7\n");
    let empty_tree = foreign_tree(&store, &[]);
    let bad_none = foreign_tree(&store, &[(b"100644", b"none", nonempty_blob)]);
    let tree_none = foreign_tree(&store, &[(b"40000", b"none", empty_tree)]);
    let extra = foreign_tree(
        &store,
        &[
            (b"100644", b"none", empty_blob),
            (b"100644", b"some", value),
        ],
    );
    let unknown = foreign_tree(&store, &[(b"100644", b"other", value)]);
    // Each malformed option shape fires a distinct rule: an empty tree is
    // never written, and the marker rules own the rest.
    let error = assert_rejected::<Option<i32>>(&store, &empty_tree, "empty option tree");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("is empty; use the none marker")),
        "{error:?}"
    );
    let error = assert_rejected::<Option<i32>>(&store, &bad_none, "nonempty none target");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("is not an empty blob")),
        "{error:?}"
    );
    let error = assert_rejected::<Option<i32>>(&store, &tree_none, "tree none target");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("none marker must have blob mode")),
        "{error:?}"
    );
    let error = assert_rejected::<Option<i32>>(&store, &extra, "multiple option entries");
    assert!(
        matches!(error, DeserializeError::MalformedOption { found: 2 }),
        "{error:?}"
    );
    let error = assert_rejected::<Option<i32>>(&store, &unknown, "unknown option marker");
    assert!(
        matches!(error, DeserializeError::MislabeledOption { ref name } if name == "other"),
        "{error:?}"
    );

    let error = assert_rejected::<()>(&store, &empty_tree, "empty unit tree");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("is empty; use the none marker")),
        "{error:?}"
    );
    assert_rejected::<Vec<i32>>(&store, &empty_tree, "empty list tree");
    let error = assert_rejected::<()>(&store, &bad_none, "nonempty unit marker");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("is not an empty blob")),
        "{error:?}"
    );
    // A sequence root reads its entries as ordinals, so a stray marker name
    // is an invalid ordinal rather than a marker complaint.
    let error = assert_rejected::<Vec<i32>>(&store, &extra, "extra list marker entry");
    assert!(
        matches!(error, DeserializeError::InvalidOrdinal(ref name) if name == "none"),
        "{error:?}"
    );
    let error = assert_rejected::<()>(&store, &extra, "extra unit marker entry");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("expected a single none marker entry")),
        "{error:?}"
    );

    // The sole child of `some` must use a canonical mode even though its
    // target object has the expected kind.
    let wrong_some_mode = foreign_tree(&store, &[(b"100755", b"some", value)]);
    assert_rejected::<Option<i32>>(&store, &wrong_some_mode, "executable some child");
    let wrong_none_mode = foreign_tree(&store, &[(b"100755", b"none", empty_blob)]);
    assert_rejected::<Option<i32>>(&store, &wrong_none_mode, "executable none marker");
}

#[test]
fn malformed_or_unknown_enum_tags_are_rejected() {
    let store = ObjectStore::default();
    for bytes in [
        b"unknown\n".as_slice(),
        b"none\n",
        b"waiting\n",
        b"%a%2Fb\n",
        b"%none",
    ] {
        assert_rejected_leaf::<Task>(&store, bytes);
    }
    let value = blob(&store, b"1\n");
    let empty_blob = blob(&store, b"");
    let marker = foreign_tree(&store, &[(b"100644", b"none", empty_blob)]);
    let unit_as_tree = foreign_tree(&store, &[(b"40000", b"%none", marker)]);
    let error = assert_rejected::<Task>(&store, &unit_as_tree, "unit variant as tree");
    assert!(
        matches!(error, DeserializeError::UnitVariantIsTree { ref variant } if variant == "none"),
        "{error:?}"
    );
    let unknown = foreign_tree(&store, &[(b"100644", b"unknown", value)]);
    let error = assert_rejected::<Task>(&store, &unknown, "unknown variant");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("unknown enum variant \"unknown\"")),
        "{error:?}"
    );
    let noncanonical = foreign_tree(&store, &[(b"100644", b"%a%2fb", value)]);
    let error = assert_rejected::<Task>(&store, &noncanonical, "noncanonical enum escape");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("invalid encoded name")),
        "{error:?}"
    );
    let multiple = foreign_tree(
        &store,
        &[(b"100644", b"%a%2Fb", value), (b"40000", b"pair", marker)],
    );
    let error = assert_rejected::<Task>(&store, &multiple, "multiple enum tags");
    assert!(
        matches!(error, DeserializeError::MalformedEnum { found: 2 }),
        "{error:?}"
    );
    let empty = foreign_tree(&store, &[]);
    let error = assert_rejected::<Task>(&store, &empty, "empty enum tree");
    assert!(
        matches!(&error, DeserializeError::Reflect(message) if message.contains("is empty; use the none marker")),
        "{error:?}"
    );
}

#[test]
fn noncanonical_integer_boolean_character_and_text_leaves_are_rejected() {
    let store = ObjectStore::default();
    for bytes in [
        b"+1\n".as_slice(),
        b"01\n",
        b"-0\n",
        b" 1\n",
        b"1 \n",
        b"1",
        b"1\n\n",
        b"2147483648\n",
    ] {
        assert_rejected_leaf::<i32>(&store, bytes);
    }
    for bytes in [b"-1\n".as_slice(), b"256\n", b"00\n", b"+0\n"] {
        assert_rejected_leaf::<u8>(&store, bytes);
    }
    for bytes in [
        b"True\n".as_slice(),
        b"FALSE\n",
        b"1\n",
        b"true",
        b"true\n\n",
    ] {
        assert_rejected_leaf::<bool>(&store, bytes);
    }
    for bytes in [b"\n".as_slice(), b"ab\n", b"\xff\n", b"x"] {
        assert_rejected_leaf::<char>(&store, bytes);
    }
    for bytes in [b"".as_slice(), b"abc", b"\xff\n"] {
        assert_rejected_leaf::<String>(&store, bytes);
    }
    for (bytes, expected) in [
        (b"0\n".as_slice(), 0i32),
        (b"-2147483648\n", i32::MIN),
        (b"2147483647\n", i32::MAX),
    ] {
        let oid = blob(&store, bytes);
        assert_fixed_point(&store, &oid, &expected);
    }
}

#[test]
fn float_golden_spellings_and_canonical_nan_and_zero() {
    for (value, expected) in [
        (1.0f64, b"1.0\n".as_slice()),
        (1e21, b"1e21\n"),
        (1e-7, b"1e-7\n"),
        (f64::INFINITY, b"inf\n"),
        (f64::NEG_INFINITY, b"-inf\n"),
    ] {
        let (root, store) = serialize(&value).expect("encode f64");
        assert_blob(&store, &root, expected);
        assert_fixed_point(&store, &root, &value);
    }
    for (value, expected) in [
        (0.1f32, b"0.1\n".as_slice()),
        (1e-7f32, b"1e-7\n"),
        (f32::INFINITY, b"inf\n"),
        (f32::NEG_INFINITY, b"-inf\n"),
    ] {
        let (root, store) = serialize(&value).expect("encode f32");
        assert_blob(&store, &root, expected);
        assert_fixed_point(&store, &root, &value);
    }
    let (zero, store) = serialize(&-0.0f64).expect("negative zero");
    assert_blob(&store, &zero, b"0.0\n");
    assert_eq!(
        deserialize::<f64>(&zero, &store).expect("zero").to_bits(),
        0
    );
    let (zero, store) = serialize(&-0.0f32).expect("negative zero");
    assert_blob(&store, &zero, b"0.0\n");
    assert_eq!(
        deserialize::<f32>(&zero, &store).expect("zero").to_bits(),
        0
    );

    let (nan, store) = serialize(&f64::from_bits(0xfff8_0000_0000_1234)).expect("nan");
    assert_blob(&store, &nan, b"nan\n");
    let decoded = deserialize::<f64>(&nan, &store).expect("decode nan");
    assert_eq!(decoded.to_bits(), 0x7ff8_0000_0000_0000);
    assert_eq!(serialize(&decoded).expect("re-encode nan").0, nan);
    let (nan, store) = serialize(&f32::from_bits(0xffc0_1234)).expect("nan");
    assert_blob(&store, &nan, b"nan\n");
    let decoded = deserialize::<f32>(&nan, &store).expect("decode nan");
    assert_eq!(decoded.to_bits(), 0x7fc0_0000);
    assert_eq!(serialize(&decoded).expect("re-encode nan").0, nan);
}

#[test]
fn noncanonical_floats_and_overflow_are_rejected() {
    let store = ObjectStore::default();
    for bytes in [
        b"1\n".as_slice(),
        b"01.0\n",
        b"+1.0\n",
        b"1.00\n",
        b"1e+21\n",
        b"1E21\n",
        b"1e-07\n",
        b"-0.0\n",
        b"-0e0\n",
        b"NaN\n",
        b"+inf\n",
        b"infinity\n",
        b"1e999\n",
        b"nan\n\n",
    ] {
        assert_rejected_leaf::<f64>(&store, bytes);
    }
    for bytes in [b"1e39\n".as_slice(), b"-0.0\n", b"NaN\n", b"1.00\n"] {
        assert_rejected_leaf::<f32>(&store, bytes);
    }
}

fn small_string() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..8).prop_map(|chars| chars.into_iter().collect())
}

fn task_strategy() -> impl Strategy<Value = Task> {
    prop_oneof![
        Just(Task::Waiting),
        Just(Task::UnitCamel),
        small_string().prop_map(Task::Note),
        proptest::collection::vec(any::<i32>(), 0..5).prop_map(Task::Wrapped),
        (any::<i32>(), any::<bool>()).prop_map(|(n, b)| Task::Pair(n, b)),
        small_string().prop_map(|label| Task::Named { label }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn generated_typed_values_encode_decode_encode_identically(
        text in small_string(),
        bytes in proptest::collection::vec(any::<u8>(), 0..16),
        numbers in proptest::collection::vec(any::<i16>(), 0..10),
        tags in proptest::collection::btree_map(small_string(), small_string(), 0..6),
        maybe in proptest::option::of(any::<i32>()),
        task in task_strategy(),
    ) {
        let value = Generated { text, bytes, numbers, tags, maybe, task };
        let (root, store) = serialize(&value).expect("encode generated value");
        let decoded: Generated = deserialize(&root, &store).expect("decode generated value");
        prop_assert_eq!(&decoded, &value);
        let (again, _) = serialize(&decoded).expect("re-encode generated value");
        prop_assert_eq!(again, root);
    }
}
