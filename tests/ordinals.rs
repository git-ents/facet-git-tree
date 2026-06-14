//! Integration tests for ordinal entry naming.
//!
//! Covers spec requirement:
//!   serialization.design.trees.ordinals
//!     — sequence elements (Array, Vec) are named by their zero-based index as
//!       zero-padded decimal, at least four digits wide (0000, 0001, …, 0010, …)
//!     — correctness MUST NOT depend on tree-entry ordering; indices are parsed
//!       numerically on read, and collections larger than 9999 remain correct.
//!
//! (Tuple-variant ordinal naming is covered in `variants.rs`.)

use facet_git_tree::serialize;

mod common;
use common::{WithArray, WithVec, get_tree_entry_mode, non_sentinel_entries, roundtrip};

/// Vec elements are named by their zero-padded, zero-based index.
#[test]
#[ignore = "serialization not yet implemented"]
fn vec_elements_named_by_zero_padded_ordinal() {
    let (root_id, store) = serialize(&WithVec {
        items: vec![10, 20, 30],
    })
    .expect("serialize ok");

    let (_, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    let names: Vec<String> = non_sentinel_entries(&store, &items_id)
        .iter()
        .map(|e| e.filename.to_string())
        .collect();

    assert!(
        names.contains(&"0000".to_string()),
        "missing 0000 in {names:?}"
    );
    assert!(
        names.contains(&"0001".to_string()),
        "missing 0001 in {names:?}"
    );
    assert!(
        names.contains(&"0002".to_string()),
        "missing 0002 in {names:?}"
    );
}

/// Array elements are named by their zero-padded, zero-based index.
#[test]
#[ignore = "serialization not yet implemented"]
fn array_elements_named_by_zero_padded_ordinal() {
    let (root_id, store) = serialize(&WithArray {
        values: [10, 20, 30, 40],
    })
    .expect("serialize ok");

    let (_, arr_id) = get_tree_entry_mode(&store, &root_id, "values");
    let names: Vec<String> = non_sentinel_entries(&store, &arr_id)
        .iter()
        .map(|e| e.filename.to_string())
        .collect();
    for expected in ["0000", "0001", "0002", "0003"] {
        assert!(
            names.contains(&expected.to_string()),
            "missing {expected} in {names:?}"
        );
    }
}

/// Ordinal names are at least four digits wide and parse as their numeric index.
#[test]
#[ignore = "serialization not yet implemented"]
fn ordinal_names_are_at_least_four_digits() {
    let (root_id, store) = serialize(&WithVec {
        items: vec![10, 20, 30],
    })
    .expect("serialize ok");

    let (_, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    for entry in non_sentinel_entries(&store, &items_id) {
        let name = entry.filename.to_string();
        assert!(
            name.len() >= 4,
            "ordinal name {name:?} must be ≥4 digits wide"
        );
        name.parse::<usize>()
            .unwrap_or_else(|_| panic!("ordinal name {name:?} must parse numerically"));
    }
}

/// A collection larger than 9999 remains correct: index 10000 needs a five-digit
/// name (`10000`), which sorts *before* `9999` lexically, so a correct roundtrip
/// proves indices are parsed numerically rather than by tree-entry order.
#[test]
#[ignore = "serialization not yet implemented"]
fn large_vec_roundtrips_with_wide_ordinals() {
    let items: Vec<i64> = (0..=10_000).map(|i| i * 2).collect();

    // The five-digit ordinal must appear for the 10000th element.
    let (root_id, store) = serialize(&WithVec {
        items: items.clone(),
    })
    .expect("serialize ok");
    let (_, items_id) = get_tree_entry_mode(&store, &root_id, "items");
    let names: Vec<String> = non_sentinel_entries(&store, &items_id)
        .iter()
        .map(|e| e.filename.to_string())
        .collect();
    assert!(
        names.contains(&"10000".to_string()),
        "missing five-digit ordinal 10000"
    );

    // …and the values come back in numeric index order despite lexical sorting.
    let recovered = roundtrip(WithVec {
        items: items.clone(),
    });
    assert_eq!(recovered, WithVec { items });
}
