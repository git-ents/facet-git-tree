//! The codec's recursion bounds, in one place.

/// Maximum tree nesting depth the value codec walks in either direction.
///
/// Kept low so it trips before a 2 MiB default thread stack overflows on
/// the deserializer's large recursive frames (tens of KB per level in a
/// debug build).
pub(crate) const MAX_VALUE_DEPTH: usize = 32;

/// Maximum nesting depth [`check_universe`](crate::normal_form) walks
/// before refusing; a schema may be recursive.
pub(crate) const MAX_UNIVERSE_DEPTH: usize = 64;
