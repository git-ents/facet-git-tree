//! The value-walk location type shared by the schema-directed writer and the
//! migration walk.
//!
//! One home instead of two byte-for-byte copies: the walks mirror each other
//! (both thread a location so an error can name the exact spot the value
//! diverged from its schema), so their path type must not be able to drift.

use core::fmt::Write as _;

/// A location within the value being written or migrated, threaded through
/// the walk so a mismatch can name exactly where it happened.
///
/// Borrowed and stack-linked, so the happy path allocates nothing; only
/// [`Path::show`] materializes a string, at the point an error is built.
pub(crate) struct Path<'a> {
    parent: Option<&'a Path<'a>>,
    seg: Seg<'a>,
}

enum Seg<'a> {
    Root,
    Field(&'a str),
    Index(usize),
}

impl<'a> Path<'a> {
    pub(crate) const ROOT: Path<'static> = Path {
        parent: None,
        seg: Seg::Root,
    };

    pub(crate) fn field<'b>(&'b self, name: &'b str) -> Path<'b> {
        Path {
            parent: Some(self),
            seg: Seg::Field(name),
        }
    }

    pub(crate) fn index<'b>(&'b self, i: usize) -> Path<'b> {
        Path {
            parent: Some(self),
            seg: Seg::Index(i),
        }
    }

    /// Render the path from the root as `$.field[0].inner`.
    pub(crate) fn show(&self) -> String {
        let mut segs = Vec::new();
        let mut cur = Some(self);
        while let Some(p) = cur {
            segs.push(&p.seg);
            cur = p.parent;
        }
        let mut s = String::from("$");
        for seg in segs.into_iter().rev() {
            match seg {
                Seg::Root => {}
                Seg::Field(name) => {
                    s.push('.');
                    s.push_str(name);
                }
                // Writing to a String is infallible.
                Seg::Index(i) => {
                    let _ = write!(s, "[{i}]");
                }
            }
        }
        s
    }
}
