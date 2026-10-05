//! The codec's recursion bounds, in one place with unambiguous names.
//!
//! Depth limits exist so no walk — typed, schema-driven, or universe-check —
//! can recurse unboundedly on a hostile, corrupt, or pathological input.
//! They live together so the same name cannot come to mean 32 in one place
//! and 64 in another.

/// The maximum tree nesting depth the value codec walks, in either
/// direction.
///
/// Bounds recursion in [`deser_into`](crate::de) and the writers so a
/// hostile or corrupt tree cannot overflow the stack. The limit must stay
/// well under what a default thread stack can hold: the typed deserializer
/// is a large recursive frame (a debug build is tens of KB per level), so a
/// 2 MiB stack — the standard library's default for spawned threads — only
/// holds a few dozen levels before overflowing. The limit is kept low
/// enough to fire first, with margin to spare. Still far deeper than any
/// practically-encoded value nests.
pub(crate) const MAX_VALUE_DEPTH: usize = 32;

/// The maximum nesting depth [`check_universe`](crate::normal_form) walks
/// before refusing.
///
/// A schema may be recursive, so the identity universe check is bounded
/// rather than relying on the graph being finite. It is looser than
/// [`MAX_VALUE_DEPTH`] in spirit — it gates which schemas are
/// registrable, not how deep a stored value may nest — but a value nested
/// deeper than the codec could read back could never be hashed stably in
/// any case.
pub(crate) const MAX_UNIVERSE_DEPTH: usize = 64;
