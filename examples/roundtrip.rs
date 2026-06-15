//! Serialize a nested value with collections, then deserialize it back.
//!
//! Run with: `cargo run --example roundtrip`

use std::collections::HashMap;

use facet::Facet;
use facet_git_tree::{deserialize, serialize};

#[derive(Debug, Facet, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Facet, PartialEq)]
struct Drawing {
    title: String,
    points: Vec<Point>,
    tags: HashMap<String, String>,
}

fn main() {
    let drawing = Drawing {
        title: "sketch".to_string(),
        points: vec![Point { x: 0.0, y: 1.5 }, Point { x: -2.0, y: 3.25 }],
        tags: HashMap::from([
            ("author".to_string(), "alice".to_string()),
            ("status".to_string(), "draft".to_string()),
        ]),
    };

    let (root, store) = serialize(&drawing).expect("serialize");
    println!("root tree: {root}");

    let decoded: Drawing = deserialize(&root, &store).expect("deserialize");
    assert_eq!(drawing, decoded);
    println!("roundtrip succeeded: {decoded:?}");
}
