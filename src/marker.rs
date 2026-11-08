//! The one-entry `none` tree used for unit, absence, and empty containers.

use gix_object::{Kind, Write};

use crate::error::SerializeError;
use crate::hash::{HashFormat, Oid};
use crate::ser::wrap_oid;
use crate::{EntryKind, EntryMode, TreeEntry};

/// The reserved tree-entry name of the presence marker.
pub(crate) const MARKER_KEY: &str = "none";

/// The reserved tree-entry name for a present option.
pub(crate) const SOME_KEY: &str = "some";

/// Write the marker tree: one entry named [`MARKER_KEY`] pointing at the
/// empty blob (content-addressed, so all markers share one blob and one tree).
pub(crate) fn write_marker_tree<F: HashFormat, W: Write + ?Sized>(
    store: &W,
) -> Result<Oid<F>, SerializeError> {
    let marker_blob: Oid<F> = wrap_oid(
        store
            .write_buf(Kind::Blob, b"")
            .map_err(SerializeError::Backend)?,
    )?;
    let entries = vec![TreeEntry {
        mode: EntryMode::from(EntryKind::Blob),
        filename: MARKER_KEY.into(),
        oid: marker_blob.into_raw(),
    }];
    wrap_oid(
        store
            .write(&gix_object::Tree { entries })
            .map_err(SerializeError::Backend)?,
    )
}
