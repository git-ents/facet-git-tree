//! Integration tests for serialization/deserialization roundtrip.
//!
//! Covers spec requirement:
//!   deserialization.roundtrip — deserialize(serialize(x)) must equal x

use facet::Facet;
use facet_git_tree::{deserialize, serialize};
use proptest::prelude::*;

mod common;
use common::{Nested, Point, roundtrip};

// --- test type specific to nested-collection roundtrips ---

#[derive(Debug, Facet, PartialEq)]
struct Matrix {
    rows: Vec<Vec<i64>>,
}

// --- tests ---

/// Nested structs roundtrip without losing inner-field data.
#[test]
fn nested_struct_roundtrip() {
    assert_eq!(
        roundtrip(Nested {
            location: Point { x: 10.0, y: -5.0 },
            label: "home".to_string(),
        }),
        Nested {
            location: Point { x: 10.0, y: -5.0 },
            label: "home".to_string(),
        }
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    #[test]
    fn bounded_nested_sequences_preserve_empty_and_non_empty_structure(
        rows in proptest::collection::vec(
            proptest::collection::vec(-20i64..20, 0..5),
            0..5,
        ),
    ) {
        let value = Matrix { rows: rows.clone() };
        let (root, store) = serialize(&value).expect("serialize");
        let recovered: Matrix = deserialize(&root, &store).expect("deserialize");
        prop_assert_eq!(recovered, value);
    }
}
