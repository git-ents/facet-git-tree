//! Serialize a simple struct into a Git tree and inspect the resulting objects.
//!
//! Run with: `cargo run --example struct_to_tree`

use facet::Facet;
use facet_git_tree::{GitObject, serialize};

#[derive(Debug, Facet)]
struct Person {
    name: String,
    age: u32,
    active: bool,
}

fn main() {
    let person = Person {
        name: "Alice".to_string(),
        age: 42,
        active: true,
    };

    let (root, store) = serialize(&person).expect("serialize");
    println!("root tree: {root}");

    // The struct is a tree; each field is an entry pointing at a blob.
    let entries = store.get_tree(&root).expect("root is a tree");
    for entry in &entries {
        let name = &entry.filename;
        let value = match store.get(&entry.oid) {
            Some(GitObject::Blob(b)) => String::from_utf8_lossy(&b.data).into_owned(),
            _ => "<tree>".to_string(),
        };
        println!("  {name} = {value} ({})", entry.oid);
    }
}
