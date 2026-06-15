//! Serialize enums and optional fields, showing how variants and `None`/`Some`
//! are encoded as trees.
//!
//! Run with: `cargo run --example enums_and_options`

use facet::Facet;
use facet_git_tree::{deserialize, serialize};

#[derive(Debug, Facet, PartialEq)]
#[repr(u8)]
enum Shape {
    Circle { radius: f64 },
    Rectangle(f64, f64),
    Empty,
}

#[derive(Debug, Facet, PartialEq)]
struct Item {
    shape: Shape,
    label: Option<String>,
}

fn show(item: Item) {
    let (root, store) = serialize(&item).expect("serialize");
    let decoded: Item = deserialize(&root, &store).expect("deserialize");
    assert_eq!(item, decoded);
    println!("{root}  <-  {decoded:?}");
}

fn main() {
    show(Item {
        shape: Shape::Circle { radius: 2.5 },
        label: Some("disk".to_string()),
    });
    show(Item {
        shape: Shape::Rectangle(3.0, 4.0),
        label: None,
    });
    show(Item {
        shape: Shape::Empty,
        label: Some("nothing".to_string()),
    });
}
