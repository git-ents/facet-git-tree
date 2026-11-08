//! Hash-format markers and the format-checked object id [`Oid`].
//!
//! Git object ids come in two incompatible formats, SHA-1 and SHA-256, and
//! [`ObjectId`] can hold either, so it carries no static guarantee about which
//! format it belongs to. [`Oid<F>`] fixes that format in the type: an object id
//! can only become an [`Oid<F>`] through [`Oid::new`], which refuses an id of
//! the wrong format. The store and wrapper types in this crate claim a format
//! through [`Tagged`], so callers pass object ids that the type system has
//! already matched to the store they are used with.

use std::fmt;
use std::marker::PhantomData;

use gix_hash::{Kind, ObjectId};
use gix_object::{Data, Find, Write};

mod sealed {
    pub trait Sealed {}
}

/// A Git object hash format, as a type-level marker.
///
/// This trait is sealed: only [`Sha1`] and [`Sha256`] implement it, so a
/// format-parameterised type has exactly two possible instantiations.
pub trait HashFormat: sealed::Sealed + Copy + 'static {
    /// The runtime hash kind this marker denotes.
    const KIND: Kind;
}

/// The SHA-1 object format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sha1 {}

/// The SHA-256 object format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sha256 {}

impl sealed::Sealed for Sha1 {}
impl sealed::Sealed for Sha256 {}

impl HashFormat for Sha1 {
    const KIND: Kind = Kind::Sha1;
}

impl HashFormat for Sha256 {
    const KIND: Kind = Kind::Sha256;
}

/// The error returned when an [`ObjectId`] does not use the requested
/// [`HashFormat`].
///
/// This is the only failure [`Oid::new`] can produce: it means a bare object id
/// of one format was presented where the other was required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("object id uses {found} but {expected} was required")]
pub struct HashFormatMismatch {
    /// The format the caller required.
    pub expected: Kind,
    /// The format the object id actually uses.
    pub found: Kind,
}

/// An object id whose hash format is fixed by the type parameter `F`.
///
/// The only way to build one from an untrusted [`ObjectId`] is [`Oid::new`],
/// which validates the format; the wrapped value is private, so an `Oid<F>`
/// always uses format `F::KIND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid<F: HashFormat> {
    raw: ObjectId,
    _format: PhantomData<F>,
}

impl<F: HashFormat> Oid<F> {
    /// Validate `raw` against `F`, yielding an [`Oid`] only when the formats
    /// agree.
    ///
    /// # Errors
    ///
    /// Returns [`HashFormatMismatch`] when `raw` uses a different Git hash
    /// format than `F`.
    pub fn new(raw: ObjectId) -> Result<Self, HashFormatMismatch> {
        if raw.kind() != F::KIND {
            return Err(HashFormatMismatch {
                expected: F::KIND,
                found: raw.kind(),
            });
        }
        Ok(Self {
            raw,
            _format: PhantomData,
        })
    }

    /// Consume the wrapper, returning the bare object id.
    pub fn into_raw(self) -> ObjectId {
        self.raw
    }

    /// Copy out the bare object id.
    pub fn to_raw(&self) -> ObjectId {
        self.raw
    }

    /// Borrow the bare object id.
    pub fn as_oid(&self) -> &gix_hash::oid {
        self.raw.as_ref()
    }

    /// The hash format this object id is guaranteed to use.
    pub fn kind(&self) -> Kind {
        F::KIND
    }
}

impl<F: HashFormat> std::ops::Deref for Oid<F> {
    type Target = ObjectId;

    fn deref(&self) -> &ObjectId {
        &self.raw
    }
}

impl<F: HashFormat> fmt::Display for Oid<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

impl<F: HashFormat> PartialEq<ObjectId> for Oid<F> {
    fn eq(&self, other: &ObjectId) -> bool {
        self.raw == *other
    }
}

impl<F: HashFormat> PartialEq<Oid<F>> for ObjectId {
    fn eq(&self, other: &Oid<F>) -> bool {
        *self == other.raw
    }
}

/// A store, or a format-claiming wrapper around one, that hashes objects in
/// format `F`.
///
/// This marker is what lets the entry points infer `F` from the store argument,
/// so `serialize_into(&value, &store)` needs no turbofish.
pub trait Tagged<F: HashFormat> {}

/// A borrowed or owned `gix` backend whose hash format the caller claims.
///
/// The concrete [`ObjectStore`](crate::ObjectStore) is already [`Tagged`]; for
/// an arbitrary repository or object database that satisfies
/// [`Find`]/[`Write`] but is not format-typed, wrap it once here:
/// `Backend::<Sha1, _>::new(odb)`. The crate then treats the wrapped backend as
/// using `F`, and the format claim is **not** statically checked — it is the
/// escape hatch, and a wrong claim surfaces as an object-format error at the
/// point where an id crosses the boundary.
pub struct Backend<F: HashFormat, B: ?Sized> {
    _format: PhantomData<F>,
    inner: B,
}

impl<F: HashFormat, B: std::fmt::Debug + ?Sized> std::fmt::Debug for Backend<F, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `inner` may be unsized (`B: ?Sized`); `&B` is `Debug` when `B` is, so
        // take the reference's reference to satisfy the `&dyn Debug` field cast.
        f.debug_struct("Backend")
            .field("inner", &&self.inner)
            .finish()
    }
}

impl<F: HashFormat, B> Backend<F, B> {
    /// Claim `inner`'s hash format as `F`.
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            _format: PhantomData,
        }
    }
}

impl<F: HashFormat, B: Find + ?Sized> Find for Backend<F, B> {
    fn try_find<'a>(
        &self,
        id: &gix_hash::oid,
        buffer: &'a mut Vec<u8>,
    ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
        self.inner.try_find(id, buffer)
    }
}

impl<F: HashFormat, B: Write + ?Sized> Write for Backend<F, B> {
    fn write_stream(
        &self,
        kind: gix_object::Kind,
        size: u64,
        from: &mut dyn std::io::Read,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.inner.write_stream(kind, size, from)
    }

    fn write_buf_with_known_id(
        &self,
        kind: gix_object::Kind,
        from: &[u8],
        id: ObjectId,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.inner.write_buf_with_known_id(kind, from, id)
    }

    fn write_stream_with_known_id(
        &self,
        kind: gix_object::Kind,
        size: u64,
        from: &mut dyn std::io::Read,
        id: ObjectId,
    ) -> Result<ObjectId, gix_object::write::Error> {
        self.inner.write_stream_with_known_id(kind, size, from, id)
    }
}

impl<F: HashFormat, B: ?Sized> Tagged<F> for Backend<F, B> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_matching_format_and_refuses_the_other() {
        let raw = ObjectId::empty_tree(Kind::Sha1);
        let oid: Oid<Sha1> = Oid::new(raw).expect("sha1 id");
        assert_eq!(oid.kind(), Kind::Sha1);
        assert_eq!(oid.into_raw(), raw);
        assert!(matches!(
            Oid::<Sha256>::new(raw),
            Err(HashFormatMismatch {
                expected: Kind::Sha256,
                found: Kind::Sha1,
            })
        ));

        let raw = ObjectId::empty_tree(Kind::Sha256);
        assert!(Oid::<Sha1>::new(raw).is_err());
        assert!(Oid::<Sha256>::new(raw).is_ok());
    }
}
