//! The identity normal form: the frozen mapping's bytes, the type-universe
//! check, and the `#[facet(facet_git_tree::identity_key)]` marker that says
//! which subtree the check applies to.

use std::collections::BTreeMap;

use facet::Facet;
use facet_git_tree::normal_form::{self, Key, NormalForm};
use facet_git_tree::{
    IDENTITY_DEF_PREFIX, Node, ObjectId, Oid, UniverseError, check_identity_subtrees,
    check_universe, identity_subtrees, schema_of, serialize,
};

/// An anchor identity: the three non-derivable coordinates, all in the
/// universe.
#[derive(Facet)]
#[facet(facet_git_tree::identity_key)]
struct Identity {
    genesis_rev: [u8; 20],
    path: String,
    span: (u64, u64),
}

#[derive(Facet)]
struct Binding {
    identity: Identity,
    hints: Hints,
}

#[derive(Facet)]
struct Hints {
    fingerprints: Vec<String>,
    descriptor: Option<String>,
}

/// A key subtree marked on the field rather than on the type, holding a value
/// the universe excludes.
#[derive(Facet)]
struct Action {
    #[facet(facet_git_tree::identity_key)]
    key: ActionKey,
}

#[derive(Facet)]
struct ActionKey {
    executor: String,
    params: Params,
}

#[derive(Facet)]
#[repr(u8)]
enum Params {
    None,
    Diff,
}

fn identity() -> NormalForm {
    NormalForm::Struct(BTreeMap::from([
        (
            "genesis_rev".to_owned(),
            NormalForm::Hash(
                ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap(),
            ),
        ),
        (
            "path".to_owned(),
            NormalForm::Str("src/refdb.rs".to_owned()),
        ),
        (
            "span".to_owned(),
            NormalForm::List(vec![NormalForm::U64(4180), NormalForm::U64(4630)]),
        ),
    ]))
}

/// Pin the frozen bare-value mapping to an independently computed Git object id.
#[test]
fn identity_hash_is_frozen() {
    let (oid, _store) = normal_form::hash(&identity()).unwrap();
    assert_eq!(oid.to_string(), "7d2884d1a44281adbdc4072e9c5e98bca4f26ea9");

    let value = Identity {
        genesis_rev: [0x11; 20],
        path: "src/refdb.rs".to_owned(),
        span: (4180, 4630),
    };
    assert_eq!(serialize(&value).unwrap().0, oid);
}

/// Likewise for one value of each scalar family, plus the empty composite.
#[test]
fn scalar_hashes_are_frozen() {
    let cases: [(NormalForm, &str); 8] = [
        (
            NormalForm::Bool(true),
            "27ba77ddaf61532f217964734769a42cdabc17dc",
        ),
        (
            NormalForm::U32(4630),
            "686eea158cb3eef1ecff4f96f87b6a3e7c2feda4",
        ),
        (
            NormalForm::I64(-1),
            "3a2e3f4984a0ee55900f8c7894844c563d2c2744",
        ),
        (
            NormalForm::F64(1.5),
            "c239c60cba28a79a295acb60a122b8c46eb57271",
        ),
        (
            NormalForm::Char('é'),
            "c6003325155f475bd7c87731607525dce73be9cf",
        ),
        (
            NormalForm::Str("src/refdb.rs".to_owned()),
            "f5c542d7957dd11b84a5febe05816dd81122a5d9",
        ),
        (
            NormalForm::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
            "7d174b13ea69e384bbc26af0ff1119aafc9abfb7",
        ),
        (
            NormalForm::List(Vec::new()),
            "13dca1c9c6e39ea21ef7a4049bfcb4a9e2628a5c",
        ),
    ];
    for (value, expected) in cases {
        let (oid, _store) = normal_form::hash(&value).unwrap();
        assert_eq!(oid.to_string(), expected, "for {value:?}");
    }
}

/// Text leaves have a trailing newline; binary leaves are stored verbatim.
#[test]
fn leaf_blobs_hold_exactly_the_frozen_bytes() {
    let (oid, store) = normal_form::hash(&NormalForm::U32(4630)).unwrap();
    assert_eq!(
        store.get_blob(&oid).expect("read object").unwrap(),
        b"4630\n"
    );

    let (oid, store) = normal_form::hash(&NormalForm::Bool(false)).unwrap();
    assert_eq!(
        store.get_blob(&oid).expect("read object").unwrap(),
        b"false\n"
    );

    let (oid, store) = normal_form::hash(&NormalForm::Str("ab".to_owned())).unwrap();
    assert_eq!(store.get_blob(&oid).expect("read object").unwrap(), b"ab\n");

    let (oid, store) = normal_form::hash(&NormalForm::Bytes(vec![0xde, 0xad, 0xbe, 0xef])).unwrap();
    assert_eq!(
        store.get_blob(&oid).expect("read object").unwrap(),
        [0xde, 0xad, 0xbe, 0xef]
    );

    let (oid, store) = normal_form::hash(&NormalForm::List(Vec::new())).unwrap();
    let entries = store.get_tree(&oid).expect("read object").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename.to_string(), "none");
    assert_eq!(entries[0].mode.kind(), facet_git_tree::EntryKind::Blob);
    assert!(
        store
            .get_blob(&Oid::new(entries[0].oid).expect("fixtures use sha1"))
            .unwrap()
            .unwrap()
            .is_empty()
    );
}

/// Lists use three-digit lowercase hexadecimal ordinals; map keys are escaped.
#[test]
fn composite_entry_names_are_frozen() {
    let (oid, store) = normal_form::hash(&NormalForm::List(vec![NormalForm::U8(0); 17])).unwrap();
    let names: Vec<String> = store
        .get_tree(&oid)
        .expect("read object")
        .unwrap()
        .iter()
        .map(|entry| entry.filename.to_string())
        .collect();
    assert_eq!(
        names,
        [
            "000", "001", "002", "003", "004", "005", "006", "007", "008", "009", "00a", "00b",
            "00c", "00d", "00e", "00f", "010"
        ]
    );

    let (oid, store) = normal_form::hash(&NormalForm::Map(BTreeMap::from([
        (Key::new("7"), NormalForm::U8(0)),
        (Key::new("11"), NormalForm::U8(1)),
        (Key::new("some"), NormalForm::U8(2)),
        (Key::new("a/b"), NormalForm::U8(3)),
    ])))
    .unwrap();
    let names: Vec<String> = store
        .get_tree(&oid)
        .expect("read object")
        .unwrap()
        .iter()
        .map(|entry| entry.filename.to_string())
        .collect();
    assert_eq!(names, ["%a%2Fb", "%some", "11", "7"]);
}

/// String keys are escaped, including names that were invalid in the old format.
#[test]
fn map_key_names_are_escaped_and_invalid_keys_are_refused() {
    for (name, expected) in [
        ("", "%"),
        ("a/b", "%a%2Fb"),
        (".", "%."),
        ("..", "%.."),
        ("_", "_"),
        ("none", "%none"),
        ("some", "%some"),
        (".schema", "%.schema"),
        ("%", "%%25"),
        ("\0", "%%00"),
        ("naïve", "%na%C3%AFve"),
    ] {
        let value = NormalForm::Map(BTreeMap::from([(Key::new(name), NormalForm::U8(0))]));
        let (oid, store) = normal_form::hash(&value).unwrap();
        let entries = store.get_tree(&oid).expect("read object").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename.to_string(), expected);
    }

    // Only string keys are representable, so the rejections that remain are
    // names whose escaped encoding exceeds the frozen 255-byte limit.
    for key in [Key::new("a".repeat(256)), Key::new("é".repeat(43))] {
        let value = NormalForm::Map(BTreeMap::from([(key, NormalForm::U8(0))]));
        assert!(matches!(
            normal_form::hash(&value),
            Err(facet_git_tree::NormalFormError::InvalidKey { .. })
        ));
    }
}

/// Struct field names follow the same escaping and encoded-length limit as keys.
#[test]
fn struct_field_names_are_escaped_and_overlong_names_are_refused() {
    for (name, expected) in [
        ("", "%"),
        ("a/b", "%a%2Fb"),
        (".", "%."),
        ("..", "%.."),
        ("_", "_"),
        ("none", "%none"),
        ("some", "%some"),
        (".schema", "%.schema"),
        ("%", "%%25"),
        ("\0", "%%00"),
        ("naïve", "%na%C3%AFve"),
    ] {
        let value = NormalForm::Struct(BTreeMap::from([(name.to_owned(), NormalForm::U8(0))]));
        let (oid, store) = normal_form::hash(&value).unwrap();
        let entries = store.get_tree(&oid).expect("read object").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename.to_string(), expected);
    }

    for name in ["a".repeat(256), "é".repeat(43)] {
        let value = NormalForm::Struct(BTreeMap::from([(name.clone(), NormalForm::U8(0))]));
        assert!(matches!(
            normal_form::hash(&value),
            Err(facet_git_tree::NormalFormError::InvalidFieldName { field }) if field == name
        ));
    }
}

#[test]
fn an_identity_shaped_schema_is_in_the_universe() {
    let doc = schema_of::<Identity>().unwrap();
    check_universe(doc.root(), doc.defs()).unwrap();
    check_identity_subtrees(&doc).unwrap();
}

/// The check names the field path and the node variant that left the universe.
#[test]
fn an_enum_leaves_the_universe() {
    let doc = schema_of::<Action>().unwrap();
    let err = check_identity_subtrees(&doc).unwrap_err();
    let UniverseError::Excluded { path, found } = err else {
        panic!("expected an exclusion, got {err}");
    };
    assert_eq!(found, "Enum");
    assert!(path.ends_with(".params"), "{path}");
}

/// `Option` is excluded: a coordinate that may be absent is a different
/// identity, not the same one with a hole.
#[test]
fn options_and_dynamic_nodes_are_excluded() {
    let doc = schema_of::<Hints>().unwrap();
    let excluded = |node: &Node| match check_universe(node, doc.defs()) {
        Err(UniverseError::Excluded { found, .. }) => found,
        other => panic!("expected an exclusion, got {other:?}"),
    };
    assert_eq!(excluded(doc.root()), "Optional");
    assert_eq!(excluded(&Node::Dynamic), "Dynamic");
    assert_eq!(excluded(&Node::RawTree), "RawTree");
    assert_eq!(excluded(&Node::Unit), "Unit");
    assert_eq!(excluded(&Node::USize), "USize");
    assert_eq!(
        excluded(&Node::Map {
            key: Box::new(Node::Unit),
            value: Box::new(Node::U8),
        }),
        "Unit"
    );
}

/// A reference with no definition is reported as such, not silently accepted.
#[test]
fn a_dangling_ref_is_reported() {
    let err = check_universe(&Node::Ref("Nope".to_owned()), &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        err,
        UniverseError::UnknownRef { ref name, .. } if name == "Nope"
    ));
}

/// The marker is compiled into the schema document as a reserved definition,
/// so it survives into whatever registers the schema.
#[test]
fn the_marker_lands_in_the_schema_document() {
    let doc = schema_of::<Binding>().unwrap();
    let marked: Vec<&str> = identity_subtrees(&doc).map(|(name, _)| name).collect();
    assert_eq!(marked, [format!("{IDENTITY_DEF_PREFIX}0").as_str()]);

    let Node::Struct(fields) = &doc.defs()["Binding"] else {
        panic!("Binding is a named-field struct");
    };
    assert_eq!(
        fields["identity"],
        Node::Ref(format!("{IDENTITY_DEF_PREFIX}0"))
    );
}

/// A reference adds no tree level, so marking a subtree cannot move a single
/// byte of the value encoding.
#[test]
fn marking_does_not_change_the_encoding() {
    #[derive(Facet)]
    struct Unmarked {
        span: (u64, u64),
    }

    #[derive(Facet)]
    struct Marked {
        #[facet(facet_git_tree::identity_key)]
        span: (u64, u64),
    }

    let (marked, _) = serialize(&Marked { span: (1, 2) }).unwrap();
    let (unmarked, _) = serialize(&Unmarked { span: (1, 2) }).unwrap();
    assert_eq!(marked, unmarked);
}
