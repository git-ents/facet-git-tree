//! Integration tests for recursive types.
//!
//! Covers spec requirement:
//!   serialization.design.schemaless
//!     — no schema is stored, so a self-referential type cannot introduce a cycle
//!       into the encoding; the value itself is finite and cycle-free.

use facet::Facet;
use facet_git_tree::serialize;

mod common;
use common::roundtrip;

/// A self-referential type: a node owns child nodes of the same type.
#[derive(Debug, Facet, PartialEq)]
struct TreeNode {
    value: i64,
    children: Vec<TreeNode>,
}

fn sample() -> TreeNode {
    TreeNode {
        value: 1,
        children: vec![
            TreeNode {
                value: 2,
                children: vec![TreeNode {
                    value: 4,
                    children: vec![],
                }],
            },
            TreeNode {
                value: 3,
                children: vec![],
            },
        ],
    }
}

/// A recursive type serializes without infinite recursion: only the finite value
/// is encoded, never the (self-referential) type definition.
#[test]
#[ignore = "serialization not yet implemented"]
fn recursive_type_serializes() {
    let (root_id, store) = serialize(&sample()).expect("recursive type must serialize");
    assert!(
        store.get(&root_id).is_some(),
        "root id must resolve in the store"
    );
}

/// A recursive value roundtrips, preserving the whole tree of nodes.
#[test]
#[ignore = "serialization not yet implemented"]
fn recursive_type_roundtrips() {
    assert_eq!(roundtrip(sample()), sample());
}
