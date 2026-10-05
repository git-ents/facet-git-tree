//! Error types for each crate operation.

use gix_hash::ObjectId;

/// A user-supplied key cannot be used as a Git tree entry name.
///
/// Tree entry names double as path segments, so a key may not contain `/`
/// or NUL, and may not equal the reserved presence-marker name (`"_"`):
/// a real entry so named would be indistinguishable from the marker on read.
#[derive(Debug, thiserror::Error)]
#[error(
    "invalid key {key:?}: not a usable git tree entry name (must be non-empty, must not contain \
     '/' or NUL, must not be \".\" or \"..\", and must not equal the reserved marker \"_\")"
)]
pub struct KeyError {
    /// The offending key.
    pub key: String,
}

/// An error produced by serialization.
#[derive(Debug, thiserror::Error)]
pub enum SerializeError {
    /// A key cannot be represented as a Git tree entry name.
    #[error(transparent)]
    Key(#[from] KeyError),
    /// An error from the underlying `gix` object backend.
    #[error("git object backend error")]
    Backend(#[source] gix_object::write::Error),
    /// A `facet` reflection operation failed.
    ///
    /// Reflection errors borrow from the reflected shape and are not
    /// `'static`-friendly, so they are collapsed to text at this boundary.
    #[error("reflection error: {0}")]
    Reflect(String),
    /// A map key's textual form is not valid UTF-8, so it cannot become a
    /// Git tree entry name.
    #[error("map key is not valid UTF-8")]
    NonUtf8MapKey,
    /// Two map pairs carry the same composite key.
    ///
    /// Pair entries are named by the pair sub-tree's object id and the pair
    /// names its key, so two entries for one key can only mean the map's key
    /// equality is broken (e.g. NaN float keys). A merged or foreign tree
    /// carrying two pairs for one key is refused on read by
    /// [`DeserializeError::DuplicateKey`] instead.
    #[error(
        "two map pairs carry the same key (key object {oid}); the map's key equality is broken"
    )]
    DuplicateKey {
        /// The shared key object id.
        oid: ObjectId,
    },
    /// The value contains a type this encoding does not support.
    #[error("unsupported type for serialization: {0}")]
    Unsupported(&'static str),
    /// The value contains a scalar type this encoding does not support.
    #[error("unsupported scalar type: {0}")]
    UnsupportedScalar(&'static str),
    /// A dynamic value holds a number with no exact textual rendering.
    ///
    /// Writing a lossy `f64` approximation would change the value and its
    /// object id, so it is refused. A `facet_value::Value` always downcasts
    /// first and renders integers exactly at any width; this remains for
    /// dynamic values of other types whose vtable cannot render exactly.
    #[error("dynamic number has no exact textual rendering")]
    UnrepresentableNumber,
    /// A dynamic value's runtime kind is not supported by this encoding.
    ///
    /// Refused rather than guessed at: `DynValueKind` is `#[non_exhaustive]`.
    #[error("unsupported dynamic value kind: {0}")]
    UnsupportedDynamicKind(String),
    /// Serialization exceeded the maximum supported nesting depth.
    #[error("maximum nesting depth ({0}) exceeded while serializing")]
    MaxDepth(usize),
}

/// An error produced by deserialization.
#[derive(Debug, thiserror::Error)]
pub enum DeserializeError {
    /// A referenced object was not present in its backing store.
    #[error("object {0} not found")]
    NotFound(ObjectId),
    /// An object was expected to be a tree but was of another kind.
    #[error("object {0} is not a tree")]
    NotATree(ObjectId),
    /// An object was expected to be a blob but was of another kind.
    #[error("object {0} is not a blob")]
    NotABlob(ObjectId),
    /// A tree entry name is not valid UTF-8.
    ///
    /// Write-side names are always UTF-8, so this can only arise from an
    /// externally-produced tree.
    #[error("tree entry name {0:?} is not valid UTF-8")]
    NonUtf8Name(String),
    /// A scalar blob's contents are not valid UTF-8.
    #[error("blob {0} is not valid UTF-8")]
    NonUtf8Blob(ObjectId),
    /// A leaf blob's final byte is not `\n`.
    ///
    /// Every leaf blob carries exactly one mandatory trailing newline; a
    /// leaf missing it is foreign or corrupt and is rejected rather than
    /// accepted leniently. The presence marker (`crate::marker`) is a
    /// separate structural object and is never checked.
    #[error(
        "leaf blob {0} is missing its mandatory trailing newline — it predates \
         the trailing-newline leaf encoding and must be re-stored"
    )]
    MissingLeafNewline(ObjectId),
    /// Deserialization exceeded the maximum supported nesting depth.
    #[error("maximum nesting depth ({0}) exceeded while deserializing")]
    MaxDepth(usize),
    /// A sequence entry name is not a valid decimal ordinal.
    #[error("invalid sequence ordinal {0:?}")]
    InvalidOrdinal(String),
    /// Two sequence entries name the same numeric ordinal (e.g. `"0"` and
    /// `"0000"`).
    #[error("duplicate sequence ordinal {0}: two entries name the same index")]
    DuplicateOrdinal(usize),
    /// An error from the underlying `gix` object backend.
    #[error("git object backend error")]
    Backend(#[source] gix_object::find::Error),
    /// A stored tree object's bytes could not be decoded as a Git tree.
    #[error("failed to decode tree {oid}")]
    Decode {
        /// The id of the undecodable object.
        oid: ObjectId,
        /// The underlying `gix` decode error.
        #[source]
        source: gix_object::decode::Error,
    },
    /// A `facet` reflection operation failed (see [`SerializeError::Reflect`]).
    #[error("reflection error: {0}")]
    Reflect(String),
    /// A scalar blob's text failed to parse as the target type.
    #[error("cannot parse {text:?} as {shape}: {reason}")]
    Parse {
        /// The type identifier of the target scalar shape.
        shape: &'static str,
        /// The text that failed to parse.
        text: String,
        /// The parse failure, collapsed to text (reflection errors are not
        /// `'static`-friendly).
        reason: String,
    },
    /// An `Option` tree does not hold exactly one entry.
    ///
    /// `Some` is exactly one entry named `some`; `None` is the marker tree,
    /// never a literal empty tree.
    #[error("malformed Option tree: expected a single \"some\" entry, found {found} entries")]
    MalformedOption {
        /// How many entries the tree actually holds.
        found: usize,
    },
    /// An `Option` tree's single entry is not named `some`.
    #[error("malformed Option tree: entry must be named \"some\", found {name:?}")]
    MislabeledOption {
        /// The entry name actually found.
        name: String,
    },
    /// A non-unit enum variant's tag tree does not hold exactly one entry.
    #[error("malformed enum tree: expected exactly one entry, found {found}")]
    MalformedEnum {
        /// How many entries the tree actually holds.
        found: usize,
    },
    /// A unit enum variant's tag object is a tree instead of a blob.
    #[error("enum variant {variant:?} is unit but its tag object is a tree, not a blob")]
    UnitVariantIsTree {
        /// The variant name.
        variant: String,
    },
    /// A non-unit enum variant's tag object is a blob instead of a tree.
    #[error("enum variant {variant:?} has a payload and must be a tree, found a blob")]
    VariantPayloadIsBlob {
        /// The variant name.
        variant: String,
    },
    /// A composite-key map pair sub-tree is missing its `k` or `v` entry.
    #[error("map pair sub-tree missing {entry:?} entry")]
    MissingMapPairEntry {
        /// The missing entry name (`"k"` or `"v"`).
        entry: &'static str,
    },
    /// A composite-key map tree holds two pairs for the same key.
    ///
    /// Either pair could be "the" value, so accepting both would silently
    /// pick one.
    #[error("map holds two pairs for the same key (key object {oid})")]
    DuplicateKey {
        /// The shared key object id.
        oid: ObjectId,
    },
    /// A struct tree lacks the entry a non-defaulted field requires.
    ///
    /// Fields with a `facet` default may be absent; an `Option` field reads
    /// as `None` when absent. Any other missing field means the tree does
    /// not describe this type.
    #[error("struct field {field:?} is missing from the tree")]
    MissingField {
        /// The field (or positional-ordinal name) the tree omits.
        field: String,
    },
    /// A tree entry has no counterpart field in the target type.
    ///
    /// Without this check a foreign tree sharing even one field name would
    /// read "successfully" while dropping its remaining entries. Schema
    /// evolution (renames, additions, removals) goes through
    /// `crate::migration`.
    #[error("tree entry {entry:?} has no counterpart in the target type")]
    UnexpectedEntry {
        /// The entry name found in the tree.
        entry: String,
    },
    /// The target type is not supported by this encoding.
    #[error("unsupported type for deserialization: {0}")]
    Unsupported(&'static str),
}

/// An error produced by writing a value through the identity normal form.
#[derive(Debug, thiserror::Error)]
pub enum NormalFormError {
    /// A map key's name form is not usable as a Git tree entry name.
    #[error("invalid normal-form map key name {key:?}: not a usable git tree entry name")]
    InvalidKey {
        /// The offending name form.
        key: String,
    },
    /// A struct's field name is not usable as a Git tree entry name.
    #[error("invalid normal-form struct field name {field:?}: not a usable git tree entry name")]
    InvalidFieldName {
        /// The offending field name.
        field: String,
    },
    /// A list holds more elements than eight-digit ordinals can name.
    ///
    /// The ordinal width is part of the frozen mapping, so a longer list is
    /// refused rather than silently widened.
    #[error(
        "normal-form list holds {len} elements, more than the {max} eight-digit ordinals can name"
    )]
    ListTooLong {
        /// The element count.
        len: usize,
        /// The largest count the mapping can name.
        max: usize,
    },
    /// An error from the underlying `gix` object backend.
    #[error("git object backend error")]
    Backend(#[source] gix_object::write::Error),
}

/// An error produced by the identity normal form's type-universe check.
///
/// Every variant names the `path` within the checked subtree.
#[derive(Debug, thiserror::Error)]
pub enum UniverseError {
    /// A node the universe excludes.
    #[error("at {path}: {found} is outside the identity normal form's universe")]
    Excluded {
        /// The location within the checked subtree.
        path: String,
        /// The excluded node variant's name.
        found: &'static str,
    },
    /// A `Node::Ref` names a definition absent from the schema document.
    #[error("at {path}: schema ref {name:?} has no definition in the document")]
    UnknownRef {
        /// The location within the checked subtree.
        path: String,
        /// The undefined reference name.
        name: String,
    },
    /// The check exceeded the maximum supported nesting depth.
    #[error("at {path}: maximum nesting depth ({depth}) exceeded while checking the universe")]
    MaxDepth {
        /// The location reached when the limit tripped.
        path: String,
        /// The limit that was exceeded.
        depth: usize,
    },
}

/// An error produced by schema generation.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The shape contains a scalar type this encoding does not support.
    #[error("unsupported scalar type in schema: {0}")]
    UnsupportedScalar(&'static str),
    /// The shape contains a type this encoding does not support.
    #[error("unsupported type in schema: {0}")]
    UnsupportedShape(&'static str),
    /// A smart pointer shape carries no pointee shape to collapse to.
    #[error("smart pointer {0} has no pointee shape")]
    MissingPointee(&'static str),
    /// The embedded kind name is not a valid Git ref-name segment.
    #[error("invalid embedded schema kind name {name:?}: {reason}")]
    InvalidKindName {
        /// The offending name.
        name: String,
        /// The first violated ref-name rule.
        reason: &'static str,
    },
    /// The schema document's embedded kind is the anonymous-root sentinel,
    /// which must be named with [`Schema::with_kind`](crate::Schema::with_kind)
    /// before publication.
    #[error(
        "schema kind is the anonymous-root sentinel; name it with `Schema::with_kind` before \
         publication"
    )]
    AnonymousKind,
    /// Schema generation exceeded the maximum supported nesting depth.
    #[error("maximum nesting depth ({0}) exceeded while generating schema")]
    MaxDepth(usize),
}

/// An error produced by the schema-schema pin.
#[derive(Debug, thiserror::Error)]
pub enum SchemaPinError {
    /// The document pins a schema-schema generation this build does not speak.
    ///
    /// An oid pin gives equality only, never ordering, so this cannot
    /// distinguish older from newer — only unrecognized.
    #[error(
        "schema tree {tree} was written against schema-schema {pinned}, which this build does \
         not recognize; it speaks {}",
        crate::schema::pin::known_generations()
    )]
    Unrecognized {
        /// The schema tree carrying the unrecognized pin.
        tree: ObjectId,
        /// The pinned schema-schema tree id.
        pinned: ObjectId,
    },
    /// The document carries no `schema` pin entry and is not itself a known
    /// schema-schema root.
    #[error("schema tree {0} carries no schema-schema pin and is not itself a known root")]
    Unpinned(ObjectId),
    /// Writing the document or its pinned schema-schema tree failed.
    #[error(transparent)]
    Serialize(#[from] SerializeError),
    /// The embedded kind name failed Git ref-segment validation.
    #[error(transparent)]
    Schema(#[from] SchemaError),
    /// A pre-`kind` generation tree lacks the exact historical representation
    /// that reader expects.
    #[error("legacy schema tree {tree} has invalid pre-kind representation: {reason}")]
    LegacyFormat {
        /// The schema tree that failed the compatibility check.
        tree: ObjectId,
        /// The violated compatibility invariant.
        reason: &'static str,
    },
    /// Reading the pin entry or the document itself failed.
    #[error(transparent)]
    Deserialize(#[from] DeserializeError),
    /// The canonical schema document did not survive the fixed-point check.
    #[error("schema fixed point failed during {stage}: expected {expected}, observed {observed}")]
    FixedPoint {
        /// The operation that produced the unexpected digest.
        stage: &'static str,
        /// The compile-time or canonical expected digest.
        expected: ObjectId,
        /// The digest produced by this build.
        observed: ObjectId,
    },
    /// The canonical schema document could not be decoded during bootstrap.
    #[error("schema fixed point decode failed: expected {expected}, observed {observed}: {source}")]
    FixedPointDecode {
        /// The compile-time canonical digest expected by this build.
        expected: ObjectId,
        /// The tree presented to the normal schema reader.
        observed: ObjectId,
        /// The underlying decode failure.
        #[source]
        source: DeserializeError,
    },
}

/// An error produced by the migration-schema pin.
///
/// Separate from [`SchemaPinError`] because a build may speak one generation
/// of `Schema` and a different generation of `Migration`.
#[derive(Debug, thiserror::Error)]
pub enum MigrationPinError {
    /// The migration pins a migration-schema generation this build does not
    /// speak.
    ///
    /// Refusing here stops an unrecognized operator from being silently
    /// skipped, which would produce a value that looks conformant and is not.
    #[error(
        "migration tree {tree} was written against migration-schema {pinned}, which this build \
         does not recognize; it speaks {}",
        crate::migration::pin::known_generations()
    )]
    Unrecognized {
        /// The migration tree carrying the unrecognized pin.
        tree: ObjectId,
        /// The pinned migration-schema tree id.
        pinned: ObjectId,
    },
    /// The migration carries no `schema` pin entry and is not itself a known
    /// migration-schema root.
    #[error("migration tree {0} carries no migration-schema pin and is not itself a known root")]
    Unpinned(ObjectId),
    /// Writing the migration or its pinned migration-schema tree failed.
    #[error(transparent)]
    Serialize(#[from] SerializeError),
    /// Reading the pin entry or the migration itself failed.
    #[error(transparent)]
    Deserialize(#[from] DeserializeError),
}

/// An error produced by schema-driven deserialization (`value` feature).
#[derive(Debug, thiserror::Error)]
pub enum SchemaReadError {
    /// The underlying tree walk failed exactly as a typed read would.
    #[error(transparent)]
    Deserialize(#[from] DeserializeError),
    /// A `Node::Ref` names a definition absent from the document's `defs`.
    #[error("schema ref {0:?} has no definition in the document")]
    UnknownRef(String),
    /// An enum tree's variant name is not present in the schema.
    #[error("unknown enum variant {variant:?}; schema defines {expected:?}")]
    UnknownVariant {
        /// The variant name found in the tree.
        variant: String,
        /// The variant names the schema defines.
        expected: Vec<String>,
    },
    /// A fixed-length sequence's entry count does not match the schema.
    #[error("sequence length mismatch: schema expects {expected} elements, tree holds {found}")]
    ArrayLenMismatch {
        /// The element count the schema requires.
        expected: usize,
        /// The entry count the tree actually holds.
        found: usize,
    },
    /// A scalar blob's text does not parse as the schema's scalar type.
    #[error("cannot parse {text:?} as schema scalar {schema}")]
    InvalidScalar {
        /// The name of the scalar schema node (e.g. `I8`, `Bool`).
        schema: &'static str,
        /// The text that failed to parse.
        text: String,
    },
    /// A tree the schema requires to be empty holds entries.
    #[error("malformed unit tree: expected no entries, found {found}")]
    MalformedUnit {
        /// How many entries the tree actually holds.
        found: usize,
    },
    /// A struct tree lacks an entry for a field the schema defines.
    ///
    /// An `Optional` field still has a present entry (the presence marker
    /// encodes `None`), so an absent entry is always an error.
    #[error("struct field {field:?} is missing from the tree")]
    MissingField {
        /// The field the schema defines and the tree omits.
        field: String,
    },
    /// A struct tree carries an entry the schema does not define.
    #[error("tree entry {entry:?} has no counterpart in the schema")]
    UnexpectedEntry {
        /// The entry name found in the tree.
        entry: String,
    },
}

/// An error produced by schema-directed serialization (`value` feature).
///
/// Every variant beyond the backend pass-through names the `path` in the
/// value where it diverged from the schema. The accepted set is exactly the
/// image of [`deserialize_value_with_schema`](crate::deserialize_value_with_schema),
/// plus the bridges a JSON-authored value needs (integer into a float field;
/// string into a `Bytes` field).
#[derive(Debug, thiserror::Error)]
pub enum SchemaWriteError {
    /// The underlying object write, key validation, or `Dynamic`-node
    /// encoding failed.
    #[error(transparent)]
    Serialize(#[from] SerializeError),
    /// The value's runtime kind does not match the schema node.
    #[error("at {path}: expected {expected}, found {found}")]
    Expected {
        /// The location within the value.
        path: String,
        /// The kind (or kinds) the schema node accepts.
        expected: &'static str,
        /// The value's actual runtime kind.
        found: &'static str,
    },
    /// A number does not fit the schema's integer type.
    #[error("at {path}: number {value} out of range for {schema}")]
    NumberOutOfRange {
        /// The location within the value.
        path: String,
        /// The integer schema node's name (e.g. `U8`).
        schema: &'static str,
        /// The offending number's textual form.
        value: String,
    },
    /// An integer has no exact representation in the schema's float type.
    ///
    /// The float-field bridge is lossless: unrepresentable integers are
    /// refused rather than rounded.
    #[error("at {path}: number has no exact {schema} representation")]
    UnrepresentableNumber {
        /// The location within the value.
        path: String,
        /// The float schema node's name (`F32` or `F64`).
        schema: &'static str,
    },
    /// An object holds a key the struct schema does not define.
    #[error("at {path}: unknown field {field:?}")]
    UnknownField {
        /// The location of the object.
        path: String,
        /// The offending key.
        field: String,
    },
    /// An object lacks a key a struct field requires.
    ///
    /// The schema-driven read requires a tree entry for every field,
    /// `Optional` included; a silent drop here would produce an unreadable
    /// tree, so it is refused before anything is written.
    #[error("at {path}: missing field {field:?}")]
    MissingField {
        /// The location of the object.
        path: String,
        /// The field the schema requires and the object omits.
        field: String,
    },
    /// A fixed-length sequence (`Tuple` or `Array`) has the wrong element
    /// count.
    #[error("at {path}: expected {expected} elements, found {found}")]
    LengthMismatch {
        /// The location of the sequence.
        path: String,
        /// The element count the schema requires.
        expected: usize,
        /// The element count the value holds.
        found: usize,
    },
    /// An enum value is not a single-member tagged object.
    #[error("at {path}: enum must be a single-member object, found {found} members")]
    MalformedEnum {
        /// The location of the value.
        path: String,
        /// How many members the object holds.
        found: usize,
    },
    /// An enum variant name is not present in the schema.
    #[error("at {path}: unknown variant {variant:?}; schema defines {expected:?}")]
    UnknownVariant {
        /// The location of the value.
        path: String,
        /// The variant name found.
        variant: String,
        /// The variant names the schema defines.
        expected: Vec<String>,
    },
    /// A `RawTree` value is not a 40-character lowercase-hex object id.
    #[error("at {path}: invalid raw tree object id {text:?}")]
    InvalidRawTree {
        /// The location of the value.
        path: String,
        /// The offending text.
        text: String,
    },
    /// A `Ref` names a definition absent from the document's `defs` table.
    #[error("at {path}: schema ref {name:?} has no definition in the document")]
    UnknownRef {
        /// The location of the value.
        path: String,
        /// The undefined reference name.
        name: String,
    },
    /// Serialization exceeded the maximum supported nesting depth.
    ///
    /// Every hop, including `Ref` resolution, counts against the limit, so a
    /// `Ref`-to-`Ref` cycle fails here rather than recursing unboundedly.
    #[error("at {path}: maximum nesting depth ({depth}) exceeded while serializing")]
    MaxDepth {
        /// The location reached when the limit tripped.
        path: String,
        /// The limit that was exceeded.
        depth: usize,
    },
}

/// An error produced by read-time migration application (`value` feature).
///
/// Walks an already-read `facet_value::Value` guided by the source `Schema`;
/// every variant names the `path` where the value diverged from that document.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The value does not match the source schema at `path`.
    #[error("at {path}: expected {expected}, found {found}")]
    Mismatch {
        /// The location within the value.
        path: String,
        /// The kind the schema node accepts.
        expected: &'static str,
        /// The value's actual runtime kind.
        found: &'static str,
    },
    /// A fixed-length sequence's element count does not match the source
    /// schema.
    ///
    /// Refused rather than truncated: an upcast that silently dropped
    /// elements would produce a value conforming to the target schema that
    /// is not the stored value.
    #[error("at {path}: expected {expected} elements, found {found}")]
    LengthMismatch {
        /// The location within the value.
        path: String,
        /// The element count the source schema requires.
        expected: usize,
        /// The element count the value holds.
        found: usize,
    },
    /// A `Node::Ref` names a definition absent from the source document.
    #[error("at {path}: schema ref {name:?} has no definition in the source document")]
    UnknownRef {
        /// The location within the value.
        path: String,
        /// The undefined reference name.
        name: String,
    },
    /// Recursion exceeded the maximum supported nesting depth.
    #[error("at {path}: maximum nesting depth ({depth}) exceeded while applying a migration")]
    MaxDepth {
        /// The location reached when the limit tripped.
        path: String,
        /// The limit that was exceeded.
        depth: usize,
    },
}
