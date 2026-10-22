//! Model a build cache as `Map<InputHash, Artifacts>` and store it as a Git
//! tree: content addressing means equal values are written once, and nested
//! collections (a map of records, each holding a `Vec` of steps) lay out as
//! nested tree objects.
//!
//! Two inputs producing byte-identical artifacts share their stored objects,
//! so identical binaries collapse to one object. The compiled output is held
//! as `Arc<[u8]>` — the type a real build system reaches for when many readers
//! share one immutable artifact buffer — which serializes to a single Git
//! blob (pointer + slice of `u8`).
//!
//! Run with: `cargo run --example build_cache`

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use facet::Facet;
use facet_git_tree::{GitObject, ObjectId, ObjectStore, deserialize, serialize};

type InputHash = String;

#[derive(Debug, Facet, PartialEq)]
struct Step {
    name: String,
    duration_ms: u32,
}

#[derive(Debug, Facet, PartialEq)]
struct Artifacts {
    binary: Arc<[u8]>,
    log: String,
    steps: Vec<Step>,
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
    if let Some(GitObject::Tree(tree)) = store.get(root).expect("read object") {
        for entry in tree.entries {
            reachable(&entry.oid, store, seen);
        }
    }
}

/// Print the object graph rooted at `oid`: trees recurse, blobs show their text.
fn dump(oid: &ObjectId, store: &ObjectStore, depth: usize) {
    let pad = "  ".repeat(depth);
    match store.get(oid).expect("read object") {
        Some(GitObject::Tree(tree)) => {
            for entry in tree.entries {
                println!("{pad}{} ({:?}):", entry.filename, entry.mode.kind());
                dump(&entry.oid, store, depth + 1);
            }
        }
        Some(GitObject::Blob(blob)) => {
            // Every leaf blob carries a mandatory trailing newline; strip the
            // one the encoder added so the dump stays one line per value.
            let text = String::from_utf8_lossy(&blob.data);
            println!("{pad}= {}", text.strip_suffix('\n').unwrap_or(&text));
        }
        _ => println!("{pad}<missing>"),
    }
}

fn main() {
    // Two cache keys whose builds produced identical artifacts, plus one that
    // differs. Each artifact carries a nested `Vec<Step>`.
    let shared = || Artifacts {
        binary: Arc::from(b"\x7fELF\x02\x01\x01\xde\xad\xbe\xef".as_slice()),
        log: "compiled in 1.2s".to_string(),
        steps: vec![
            Step {
                name: "parse".to_string(),
                duration_ms: 120,
            },
            Step {
                name: "codegen".to_string(),
                duration_ms: 900,
            },
        ],
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
                    steps: vec![Step {
                        name: "parse".to_string(),
                        duration_ms: 110,
                    }],
                },
            ),
        ]),
    };

    let (root, store) = serialize(&cache).expect("serialize");
    println!("cache root tree: {root}\n");

    println!("object graph:");
    dump(&root, &store, 0);
    println!();

    // The map's hash-keyed entries point at each Artifacts sub-tree; the two
    // shared builds point at the same OID.
    let fields = store
        .get_tree(&root)
        .expect("read object")
        .expect("root is a tree");
    let entries = store
        .get_tree(&fields[0].oid)
        .expect("read object")
        .expect("entries map is a tree");
    for entry in &entries {
        println!("  {} -> {}", entry.filename, entry.oid);
    }

    let mut seen = HashSet::new();
    reachable(&root, &store, &mut seen);
    println!(
        "\n3 cache entries (2 identical) stored in {} distinct objects",
        seen.len()
    );
    println!("dedup: the shared Artifacts blobs and sub-tree exist only once");

    // The Arc<[u8]> binaries rebuild byte-for-byte on the way back out.
    let decoded: BuildCache = deserialize(&root, &store).expect("deserialize");
    assert_eq!(cache, decoded);
    println!("roundtrip succeeded: Arc<[u8]> artifacts restored intact");
}
