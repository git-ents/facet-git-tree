//! Integration tests for recursive (potentially cyclic) type schemas.
//!
//! Covers spec requirement:
//!   serialization.design.trees.shape
//!     — definitions are recorded in a flattened `defs` subtree that references
//!       identifiers, never a full schema tree, to avoid cycles in type definitions.
//!
//! A self-referential type is the case that *requires* that indirection: inlining
//! its definition would recurse forever.

use facet::Facet;
use facet_git_tree::{EntryKind, serialize};

mod common;
use common::{find_entry, roundtrip};

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

/// A recursive type serializes without infinite recursion and produces a schema object.
#[test]
#[ignore = "serialization not yet implemented"]
fn recursive_type_serializes() {
    let (root_id, store) = serialize(&sample()).expect("recursive type must serialize");
    let schema = find_entry(&store, &root_id, ".schema");
    assert_eq!(
        schema.mode.kind(),
        EntryKind::Tree,
        "`.schema` must be a tree"
    );
}

/// The recursive type's `.schema/defs` is flat: the self-reference is recorded by
/// identifier, never inlined as a nested schema tree (which would never terminate).
#[test]
#[ignore = "serialization not yet implemented"]
fn recursive_type_defs_is_flat() {
    let (root_id, store) = serialize(&sample()).expect("recursive type must serialize");

    let schema = find_entry(&store, &root_id, ".schema");
    let defs = find_entry(&store, &schema.oid, "defs");
    assert_eq!(
        defs.mode.kind(),
        EntryKind::Tree,
        "`.schema/defs` must be a tree"
    );

    let entries = store.get_tree(&defs.oid).expect("`defs` must be in store");
    for entry in entries {
        assert_eq!(
            entry.mode.kind(),
            EntryKind::Blob,
            "`defs` entry `{}` must be a blob (self-reference recorded by id, not inlined)",
            entry.filename
        );
    }
}

/// A recursive value roundtrips, preserving the whole tree of nodes.
#[test]
#[ignore = "serialization not yet implemented"]
fn recursive_type_roundtrips() {
    assert_eq!(roundtrip(sample()), sample());
}
