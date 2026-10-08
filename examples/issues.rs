//! Store a tracker issue — a state enum, a `Vec` of labels, optional assignee,
//! and threaded comments — as a Git tree, then round-trip it back.
//!
//! Because objects are content-addressed, two issues with identical contents
//! hash to the same root OID. This is the property that lets you dedup, diff,
//! and detect "nothing changed" for free.
//!
//! Run with: `cargo run --example issues`

use facet::Facet;
use facet_git_tree::{deserialize, serialize};

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum State {
    Open,
    Closed,
}

#[derive(Debug, Facet, PartialEq)]
struct Comment {
    author: String,
    body: String,
    replies: Vec<Comment>,
}

#[derive(Debug, Facet, PartialEq)]
struct Issue {
    title: String,
    state: State,
    labels: Vec<String>,
    assignee: Option<String>,
    comments: Vec<Comment>,
}

fn sample() -> Issue {
    Issue {
        title: "Serialize fails on composite map keys".to_string(),
        state: State::Open,
        labels: vec!["bug".to_string(), "serde".to_string()],
        assignee: Some("alice".to_string()),
        comments: vec![Comment {
            author: "bob".to_string(),
            body: "Can you share a repro?".to_string(),
            replies: vec![Comment {
                author: "alice".to_string(),
                body: "Added one to the description.".to_string(),
                replies: vec![],
            }],
        }],
    }
}

fn main() {
    let issue = sample();

    let (root, store) = serialize(&issue).expect("serialize");
    println!("issue root tree: {root}");

    let decoded: Issue = deserialize(&root, &store).expect("deserialize");
    assert_eq!(issue, decoded);
    println!("roundtrip succeeded");

    // Content addressing: an identical issue serializes to the identical root
    // OID, even through a separate store with no shared state.
    let (root_again, _) = serialize(&sample()).expect("serialize");
    assert_eq!(root, root_again);
    println!("identical issue -> identical root OID: {root_again}");
}
