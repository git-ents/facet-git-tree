//! The one-entry `none` tree used for unit, absence, and empty containers.

use gix_object::{Kind, Write};

use crate::error::SerializeError;
use crate::{EntryKind, EntryMode, ObjectId, TreeEntry};

/// The reserved tree-entry name of the presence marker.
pub(crate) const MARKER_KEY: &str = "none";

/// The reserved tree-entry name for a present option.
pub(crate) const SOME_KEY: &str = "some";

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
