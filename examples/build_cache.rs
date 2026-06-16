//! Model a build cache as a `Map<InputHash, Artifacts>` and store it as a Git
//! tree. Two inputs that produce byte-identical artifacts share their stored
//! objects: content addressing means equal values are written once.
//!
//! The example proves it by walking the resulting tree and counting the
//! distinct objects actually stored.
//!
//! Run with: `cargo run --example build_cache`

use std::collections::{HashMap, HashSet};

use facet::Facet;
use facet_git_tree::{GitObject, ObjectId, ObjectStore, serialize};

type InputHash = String;

#[derive(Debug, Facet, PartialEq)]
struct Artifacts {
    binary: String,
    log: String,
}

#[derive(Debug, Facet, PartialEq)]
struct BuildCache {
    entries: HashMap<InputHash, Artifacts>,
}

/// Collect every object reachable from `root`, following tree entries.
fn reachable(root: &ObjectId, store: &ObjectStore, seen: &mut HashSet<ObjectId>) {
    if !seen.insert(*root) {
        return;
    }
    if let Some(GitObject::Tree(tree)) = store.get(root) {
        for entry in tree.entries {
            reachable(&entry.oid, store, seen);
        }
    }
}

fn main() {
    // Two cache keys whose builds produced identical artifacts, plus one that
    // differs.
    let shared = || Artifacts {
        binary: "ELF...deadbeef".to_string(),
        log: "compiled in 1.2s".to_string(),
    };
    let cache = BuildCache {
        entries: HashMap::from([
            ("input-aaaa".to_string(), shared()),
            ("input-bbbb".to_string(), shared()),
            (
                "input-cccc".to_string(),
                Artifacts {
                    binary: "ELF...cafef00d".to_string(),
                    log: "compiled in 0.9s".to_string(),
                },
            ),
        ]),
    };

    let (root, store) = serialize(&cache).expect("serialize");
    println!("cache root tree: {root}");

    // The two shared entries point at the same Artifacts sub-tree OID.
    let entries = store.get_tree(&root).expect("root is a tree");
    let entries = store
        .get_tree(&entries[0].oid)
        .expect("entries map is a tree");
    for entry in &entries {
        println!("  {} -> {}", entry.filename, entry.oid);
    }

    let mut seen = HashSet::new();
    reachable(&root, &store, &mut seen);
    println!(
        "3 cache entries (2 identical) stored in {} distinct objects",
        seen.len()
    );
    println!("dedup: the shared Artifacts blobs and sub-tree exist only once");
}
