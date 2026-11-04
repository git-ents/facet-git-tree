//! Entry-name validation and escaping for bare typed maps.

use std::collections::HashMap;

use facet_git_tree::{KeyError, ObjectId, ObjectStore, check_key, deserialize, serialize};
use gix_object::{Kind, Write as _};
use rstest::rstest;

/// User names are accepted when their canonical encoded names fit the limit.
#[rstest]
#[case("name", "name")]
#[case("author.name", "author.name")]
#[case("000", "000")]
#[case("_", "_")]
#[case("none", "%none")]
#[case("some", "%some")]
#[case(".schema", "%.schema")]
#[case(".value", "%.value")]
#[case(".", "%.")]
#[case("..", "%..")]
#[case(".git", "%.git")]
#[case("a/b", "%a%2Fb")]
#[case("a b", "%a%20b")]
// The prose specifies a '%' prefix for every escaped name; the example in
// format.adoc omits that prefix for this one case.
#[case("naïve", "%na%C3%AFve")]
#[case("", "%")]
#[case("a\0b", "%a%00b")]
#[case("%", "%%25")]
fn accepts_and_encodes_user_names(#[case] key: &str, #[case] encoded: &str) {
    assert!(check_key(key).is_ok(), "{key:?} should be representable");

    let input = HashMap::from([(key.to_owned(), "value".to_owned())]);
    let (root, store) = serialize(&input).expect("encode typed map");
    let entries = store.get_tree(&root).expect("read map").expect("tree");
    assert_eq!(entries.len(), 1);
    let actual: &[u8] = entries[0].filename.as_ref();
    assert_eq!(actual, encoded.as_bytes());
    assert_eq!(entries[0].mode.value(), 0o100644);
    assert_eq!(
        store.get_blob(&entries[0].oid).expect("read value"),
        Some(b"value\n".to_vec())
    );
    let decoded: HashMap<String, String> = deserialize(&root, &store).expect("decode typed map");
    assert_eq!(decoded, input);
    let (reencoded, _) = serialize(&decoded).expect("re-encode typed map");
    assert_eq!(reencoded, root);
}

#[test]
fn encoded_name_limit_counts_prefix_and_percent_encoding() {
    let valid = [
        "a".repeat(255),
        format!(".{}", "a".repeat(253)),
        "/".repeat(84),
    ];
    for key in valid {
        assert!(check_key(&key).is_ok(), "{key:?} should fit");
        let input = HashMap::from([(key, "value".to_owned())]);
        let (root, store) = serialize(&input).expect("encode boundary name");
        let entries = store.get_tree(&root).expect("read map").expect("tree");
        assert!(entries[0].filename.len() <= 255);
        let decoded: HashMap<String, String> = deserialize(&root, &store).expect("decode map");
        assert_eq!(decoded, input);
    }

    let invalid = [
        "a".repeat(256),
        format!(".{}", "a".repeat(254)),
        "/".repeat(85),
    ];
    for key in invalid {
        assert!(
            matches!(check_key(&key), Err(KeyError { key: ref actual }) if actual == &key),
            "{key:?} should exceed the encoded limit"
        );
        let input = HashMap::from([(key, "value".to_owned())]);
        assert!(
            serialize(&input).is_err(),
            "overlong map key must be rejected"
        );
    }
}

#[test]
fn distinct_user_names_do_not_collide_with_structural_markers_or_each_other() {
    let input: HashMap<String, String> = [
        "", "%", "none", "some", ".schema", ".value", "a/b", "a%2Fb", "_",
    ]
    .into_iter()
    .map(|key| (key.to_owned(), format!("value {key}")))
    .collect();
    let (root, store) = serialize(&input).expect("encode map");
    let entries = store.get_tree(&root).expect("read map").expect("tree");
    let names: Vec<&[u8]> = entries
        .iter()
        .map(|entry| entry.filename.as_ref())
        .collect();
    assert_eq!(
        names,
        [
            b"%".as_slice(),
            b"%%25",
            b"%.schema",
            b"%.value",
            b"%a%252Fb",
            b"%a%2Fb",
            b"%none",
            b"%some",
            b"_"
        ]
    );
    let decoded: HashMap<String, String> = deserialize(&root, &store).expect("decode map");
    assert_eq!(decoded, input);
}

fn foreign_map(name: &[u8]) -> (ObjectId, ObjectStore) {
    let store = ObjectStore::default();
    let value = store
        .write_buf(Kind::Blob, b"value\n")
        .expect("write value");
    let mut bytes = b"100644 ".to_vec();
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.extend_from_slice(value.as_bytes());
    let root = store.write_buf(Kind::Tree, &bytes).expect("write raw tree");
    (root, store)
}

#[test]
fn rejects_noncanonical_foreign_names_and_structural_names() {
    for name in [
        b"%74itle".as_slice(),
        b"%title",
        b"%a%2fb",
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
    ] {
        let (root, store) = foreign_map(name);
        assert!(
            deserialize::<HashMap<String, String>>(&root, &store).is_err(),
            "accepted foreign entry name {name:?}"
        );
    }
    let (root, store) = foreign_map(&vec![b'x'; 256]);
    assert!(deserialize::<HashMap<String, String>>(&root, &store).is_err());
}
