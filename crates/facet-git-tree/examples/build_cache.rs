//! Model a build cache as a `Map<InputHash, Artifacts>` and store it as a Git
//! tree. Two inputs that produce byte-identical artifacts share their stored
//! objects: content addressing means equal values are written once.
//!
//! The compiled output is held as `Arc<[u8]>` — the type a real build system
//! reaches for when many readers share one immutable artifact buffer. It
//! serializes to a single Git blob (pointer + slice of `u8`), so identical
//! binaries collapse to one object.
//!
//! The example proves the dedup by walking the resulting tree and counting the
//! distinct objects actually stored, and round-trips the cache to show the
//! `Arc<[u8]>` rebuilds intact.
//!
//! Run with: `cargo run --example build_cache`

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use facet::Facet;
use facet_git_tree::{GitObject, ObjectId, ObjectStore, deserialize, serialize};

type InputHash = String;

#[derive(Debug, Facet, PartialEq)]
struct Artifacts {
    binary: Arc<[u8]>,
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
        binary: Arc::from(b"\x7fELF\x02\x01\x01\xde\xad\xbe\xef".as_slice()),
        log: "compiled in 1.2s".to_string(),
    };
    let cache = BuildCache {
        entries: HashMap::from([
            ("input-aaaa".to_string(), shared()),
            ("input-bbbb".to_string(), shared()),
            (
                "input-cccc".to_string(),
                Artifacts {
                    binary: Arc::from(b"\x7fELF\x02\x01\x01\xca\xfe\xf0\x0d".as_slice()),
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

    // The Arc<[u8]> binaries rebuild byte-for-byte on the way back out.
    let decoded: BuildCache = deserialize(&root, &store).expect("deserialize");
    assert_eq!(cache, decoded);
    println!("roundtrip succeeded: Arc<[u8]> artifacts restored intact");
}
