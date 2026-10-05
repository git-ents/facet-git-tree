//! A one-entry tree (`"_"` → empty blob) written in place of a literal empty
//! tree for `None`, dynamic `Null`, and empty collections, so a field going
//! empty is visible to git's blob-oriented `ls-tree -r` and `diff` instead of
//! disappearing. The name is reserved: `crate::check_key` rejects it for
//! dynamic keys, and the schema-directed writer validates field names too,
//! since a hand-authored [`crate::Schema`] is not bound by Rust identifiers.

use gix_object::{Kind, Write};

use crate::error::SerializeError;
use crate::{EntryKind, EntryMode, ObjectId, TreeEntry};

/// The reserved tree-entry name of the presence marker.
pub(crate) const MARKER_KEY: &str = "_";

/// Whether `entries` is exactly the marker entry.
pub(crate) fn is_marker(entries: &[(String, ObjectId, EntryKind)]) -> bool {
    matches!(entries, [(name, _, EntryKind::Blob)] if name == MARKER_KEY)
}

/// Write the marker tree: one entry named [`MARKER_KEY`] pointing at the
/// empty blob (content-addressed, so all markers share one blob and one tree).
pub(crate) fn write_marker_tree<W: Write + ?Sized>(store: &W) -> Result<ObjectId, SerializeError> {
    let marker_blob = store
        .write_buf(Kind::Blob, b"")
        .map_err(SerializeError::Backend)?;
    store
        .write(&gix_object::Tree {
            entries: vec![TreeEntry {
                mode: EntryMode::from(EntryKind::Blob),
                filename: MARKER_KEY.into(),
                oid: marker_blob,
            }],
        })
        .map_err(SerializeError::Backend)
}
