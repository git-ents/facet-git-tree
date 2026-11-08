//! Self-hosted schemas: [`Node`]/[`Schema`] describe how a `Facet` type
//! is encoded, and are themselves ordinary `Facet` values stored through this
//! crate's own tree encoding.
//!
//! [`schema_of`] converts a `Facet` type's shape into a [`Schema`]. With the
//! `value` cargo feature, a schema drives schema-directed reads and writes.

#[cfg(feature = "value")]
pub(crate) mod path;

#[cfg(feature = "value")]
pub mod read;
#[cfg(feature = "value")]
pub mod write;

use std::collections::{BTreeMap, HashMap};

use facet::{ConstTypeId, Def, Facet, ScalarType, Shape};

use crate::attr;
use crate::classify::{ShapeClass, classify};
use crate::de::{MAX_DECODE_VISITS, collapse_shape};
use crate::error::{DeserializeError, SchemaError, SchemaReadError};
use crate::limits::MAX_VALUE_DEPTH;
use crate::normal_form::IDENTITY_DEF_PREFIX;
use crate::ordinal::MAX_TREE_ENTRIES;

#[cfg(feature = "value")]
use facet_value::Value;

/// A complete, self-contained schema document.
///
/// `kind` is an opaque label, not a type check. `root` describes the value,
/// and `defs` holds the named types referenced by [`Node::Ref`].
///
/// This is the wire/`Facet` type: it is what is decoded from and encoded to
/// the store, and its fields are private so a structurally ill-formed
/// document must be built through [`Schema::new`]. Being able to *hold* a
/// `Schema` is not, however, a claim that it is well-formed — a decoded or
/// externally-sourced document may not be. Schema-directed decoding and
/// encoding therefore take a [`CheckedSchema`], the type that carries the
/// proven-well-formed fact.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Schema {
    /// An opaque entity-kind label; it is not used to decode the value.
    kind: String,
    /// The schema of the value itself. Named user types appear as
    /// [`Node::Ref`] nodes resolved through `defs`.
    root: Node,
    /// The definition table for named user types, keyed by assigned name.
    defs: BTreeMap<String, Node>,
}

/// A [`Schema`] proven well-formed by a full validation pass.
///
/// Borrows the document it validated: the value is a zero-copy view that can
/// only be obtained through [`Schema::checked`] or [`TryFrom<&Schema>`], both
/// of which run the well-formedness walk exactly once. Schema-directed
/// entry points ([`deserialize_value_with_schema`], [`validate_with_schema`],
/// [`serialize_value_with_schema`]) take a `&CheckedSchema` rather than a
/// `&Schema`, so a document that reached them has provably passed validation
/// and is not validated again.
///
/// [`deserialize_value_with_schema`]: crate::deserialize_value_with_schema
/// [`validate_with_schema`]: crate::validate_with_schema
/// [`serialize_value_with_schema`]: crate::serialize_value_with_schema
#[derive(Debug, Clone, Copy)]
pub struct CheckedSchema<'a> {
    schema: &'a Schema,
}

impl<'a> CheckedSchema<'a> {
    /// The validated document behind this token.
    pub fn schema(&self) -> &'a Schema {
        self.schema
    }
}

impl<'a> TryFrom<&'a Schema> for CheckedSchema<'a> {
    type Error = SchemaReadError;

    fn try_from(schema: &'a Schema) -> Result<Self, SchemaReadError> {
        validate_well_formed(schema)?;
        Ok(CheckedSchema { schema })
    }
}

/// Validate a schema document independently of any value.
///
/// Rejects a dangling [`Node::Ref`], a map key that is not literally
/// [`Node::String`], an oversized container, an invalid entry name, nesting
/// deeper than [`MAX_VALUE_DEPTH`], and — through [`scan_guards`] — an
/// unguarded reference cycle. Guarded contents (`List`, `Array`, `Map`,
/// `Optional`, `Enum`) break cycles but are still checked for the other
/// violations. This is the one walk that produces a [`CheckedSchema`], so its
/// accept/reject decisions are the schema-directed contract.
fn validate_well_formed(doc: &Schema) -> Result<(), SchemaReadError> {
    check_schema_len(doc.defs.len(), "definitions")?;
    let mut pending = vec![(&doc.root, 0usize)];
    for (name, node) in &doc.defs {
        check_schema_name(name)?;
        pending.push((node, 0));
    }

    let mut visited = 0usize;
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_VALUE_DEPTH {
            return Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH).into());
        }
        visited += 1;
        if visited > MAX_DECODE_VISITS {
            return Err(invalid_schema("schema node limit exceeded"));
        }
        let next = depth + 1;
        match node {
            Node::Ref(name) => {
                if !doc.defs.contains_key(name) {
                    return Err(SchemaReadError::UnknownRef(name.clone()));
                }
            }
            Node::List(elem) | Node::Optional(elem) => pending.push((elem, next)),
            Node::Array { elem, len } => {
                check_schema_len(*len, "array length")?;
                pending.push((elem, next));
            }
            Node::Map { key, value } => {
                if !matches!(key.as_ref(), Node::String) {
                    return Err(invalid_schema("map key must be literally String"));
                }
                pending.push((key, next));
                pending.push((value, next));
            }
            Node::Tuple(elems) => {
                check_schema_len(elems.len(), "tuple arity")?;
                pending.extend(elems.iter().map(|elem| (elem, next)));
            }
            Node::Struct(fields) => {
                check_schema_fields(fields, &mut pending, next)?;
            }
            Node::Enum(variants) => {
                check_schema_len(variants.len(), "enum variants")?;
                for (name, kind) in variants {
                    check_schema_name(name)?;
                    match kind {
                        VariantKind::Unit => {}
                        VariantKind::Tuple(elems) => {
                            check_schema_len(elems.len(), "tuple variant arity")?;
                            pending.extend(elems.iter().map(|elem| (elem, next)));
                        }
                        VariantKind::Struct(fields) => {
                            check_schema_fields(fields, &mut pending, next)?;
                        }
                    }
                }
            }
            Node::Unit
            | Node::Bool
            | Node::Char
            | Node::String
            | Node::I8
            | Node::I16
            | Node::I32
            | Node::I64
            | Node::I128
            | Node::ISize
            | Node::U8
            | Node::U16
            | Node::U32
            | Node::U64
            | Node::U128
            | Node::USize
            | Node::F32
            | Node::F64
            | Node::Bytes
            | Node::Cbor
            | Node::RawTree
            | Node::Dynamic => {}
        }
    }

    match scan_guards(&doc.defs) {
        GuardScan::Productive => Ok(()),
        GuardScan::Dangling(name) => Err(SchemaReadError::UnknownRef(name.to_owned())),
        GuardScan::Cyclic(cycle) => Err(invalid_schema(format!(
            "unguarded reference cycle involving {:?}",
            cycle.entered
        ))),
    }
}

fn check_schema_fields<'a>(
    fields: &'a BTreeMap<String, Node>,
    pending: &mut Vec<(&'a Node, usize)>,
    depth: usize,
) -> Result<(), SchemaReadError> {
    check_schema_len(fields.len(), "struct fields")?;
    for (name, node) in fields {
        check_schema_name(name)?;
        pending.push((node, depth));
    }
    Ok(())
}

fn check_schema_len(len: usize, what: &str) -> Result<(), SchemaReadError> {
    if len > MAX_TREE_ENTRIES {
        return Err(invalid_schema(format!(
            "{what} has {len} entries, exceeding {MAX_TREE_ENTRIES}"
        )));
    }
    Ok(())
}

fn check_schema_name(name: &str) -> Result<(), SchemaReadError> {
    crate::name::escape(name)
        .map(|_| ())
        .map_err(|error| invalid_schema(format!("invalid name {name:?}: {error}")))
}

pub(crate) fn invalid_schema(reason: impl std::fmt::Display) -> SchemaReadError {
    DeserializeError::Reflect(format!("invalid schema: {reason}")).into()
}

/// The definition a reference-cycle scan re-entered while it was still active.
struct GuardCycle<'a> {
    /// The definition whose traversal first reached the cycle.
    root: &'a str,
    /// The definition the traversal re-entered.
    entered: &'a str,
}

/// The outcome of scanning a definition table for the one structural
/// invariant shared by schema generation and schema-directed decoding: every
/// definition must reach a concrete node without crossing an unguarded
/// `Ref`/`Struct`/`Tuple` edge back into a definition already on the path.
///
/// `List`, `Array`, `Map`, `Optional`, and `Enum` are guards: their contents
/// add a tree level, so a cycle through them is productive and permitted. The
/// scan is stated here once and mapped to each caller's error type, so the
/// invariant cannot drift between generation and validation.
enum GuardScan<'a> {
    /// Every definition is productive through guarded edges.
    Productive,
    /// A definition re-enters one already on the current path.
    Cyclic(GuardCycle<'a>),
    /// A `ref` names a definition absent from the table.
    Dangling(&'a str),
}

/// Scan `defs` for an unguarded reference cycle or a dangling reference.
///
/// Iterative so a long alias chain cannot exhaust the call stack. `root` is
/// the definition whose traversal detected the cycle; callers that label the
/// offending definition by its originating type use it, callers that name the
/// re-entered definition use `entered`.
fn scan_guards(defs: &BTreeMap<String, Node>) -> GuardScan<'_> {
    enum Visit<'a> {
        Enter(&'a str),
        Exit(&'a str),
        Node(&'a Node),
    }
    // absent = unseen, false = active, true = complete.
    let mut states: BTreeMap<&str, bool> = BTreeMap::new();
    for name in defs.keys() {
        let root = name.as_str();
        let mut stack = vec![Visit::Enter(root)];
        while let Some(visit) = stack.pop() {
            match visit {
                Visit::Enter(name) => match states.get(name) {
                    Some(true) => {}
                    Some(false) => {
                        return GuardScan::Cyclic(GuardCycle {
                            root,
                            entered: name,
                        });
                    }
                    None => {
                        let Some(node) = defs.get(name) else {
                            return GuardScan::Dangling(name);
                        };
                        states.insert(name, false);
                        stack.push(Visit::Exit(name));
                        stack.push(Visit::Node(node));
                    }
                },
                Visit::Exit(name) => {
                    states.insert(name, true);
                }
                Visit::Node(node) => match node {
                    Node::Ref(name) => stack.push(Visit::Enter(name)),
                    Node::Struct(fields) => stack.extend(fields.values().map(Visit::Node)),
                    Node::Tuple(elems) => stack.extend(elems.iter().map(Visit::Node)),
                    // Each of these is a guard, or has no outgoing edges.
                    _ => {}
                },
            }
        }
    }
    GuardScan::Productive
}

/// A single schema node: the shape of one value in the encoding.
///
/// Scalar variants are per-width unit variants mirroring [`facet::ScalarType`].
/// The on-disk form of this type is a public contract; changing it is a
/// semver-major break.
#[derive(Debug, Clone, PartialEq, Facet)]
#[facet(rename_all = "lowercase")]
#[repr(u8)]
pub enum Node {
    /// The unit type `()` or a unit struct: the `none` marker tree.
    Unit,
    /// `bool`: a `true`/`false` blob.
    Bool,
    /// `char`: a blob holding the character's UTF-8 form.
    Char,
    /// A string (`String`, `&str`, `Cow<str>`): a blob of its UTF-8 bytes.
    String,
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i64`.
    I64,
    /// `i128`.
    I128,
    /// `isize`, spec'd i64-shaped: encoded as its decimal text within the
    /// i64 range, so object ids never depend on pointer width.
    ISize,
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`.
    U64,
    /// `u128`.
    U128,
    /// `usize`, spec'd u64-shaped: encoded as its decimal text within the
    /// u64 range, so object ids never depend on pointer width.
    USize,
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// A byte sequence (`Vec<u8>`, `[u8; N]`, `[u8]`): a single binary blob.
    Bytes,
    /// An opaque RFC 8949 core-deterministic CBOR payload.
    Cbor,
    /// A [`RawTree`](crate::RawTree): a verbatim tree reference.
    RawTree,
    /// A dynamic value (`facet_value::Value`): shape decided at runtime.
    Dynamic,
    /// A variable-length sequence (`Vec<T>`, `[T]`): an ordinal-named tree.
    List(Box<Node>),
    /// A fixed-length array `[T; N]`: an ordinal-named tree of exactly `len`
    /// entries.
    Array {
        /// The element schema.
        elem: Box<Node>,
        /// The exact element count, at most 4096.
        len: usize,
    },
    /// A string-keyed map: a tree named by escaped keys.
    Map {
        /// The key schema, which must be [`Node::String`].
        key: Box<Node>,
        /// The value schema.
        value: Box<Node>,
    },
    /// An `Option<T>`: the presence-marker tree (`crate::marker`) for `None`,
    /// a single `some` entry for `Some`.
    Optional(Box<Node>),
    /// A tuple or tuple struct: a tree with ordinal-named entries.
    Tuple(Vec<Node>),
    /// A named-field struct: a tree with one entry for every field.
    Struct(BTreeMap<String, Node>),
    /// An externally tagged enum, keyed by variant name.
    Enum(BTreeMap<String, VariantKind>),
    /// A reference to a named definition in the document's `defs` table.
    Ref(String),
}

impl Node {
    /// Follow `ref` nodes through `defs` to the concrete node they name.
    ///
    /// `ref` nodes are transparent on disk: resolving one adds no tree level
    /// and no value, so a chain of aliases is equivalent to its target.
    /// Returns `self` when it is already concrete.
    ///
    /// A chain longer than the definition table must repeat a name and so can
    /// never reach a concrete node; that is reported as [`RefError::Cycle`]
    /// instead of being walked forever.
    pub fn resolve<'a>(&'a self, defs: &'a BTreeMap<String, Node>) -> Result<&'a Node, RefError> {
        let mut node = self;
        let mut consumed = 0usize;
        while let Node::Ref(name) = node {
            let target = defs
                .get(name)
                .ok_or_else(|| RefError::Dangling { name: name.clone() })?;
            consumed += 1;
            if consumed > defs.len() {
                return Err(RefError::Cycle { name: name.clone() });
            }
            node = target;
        }
        Ok(node)
    }

    /// The keyword naming this node in a schema.
    ///
    /// These are exactly the lowercase variant names: the leaf keywords
    /// `unit`, `bool`, `char`, `string`, the integer and float widths,
    /// `bytes`, `cbor`, `rawtree`, `dynamic`; and the composite keywords
    /// `list`, `array`, `map`, `optional`, `tuple`, `struct`, `enum`, `ref`.
    /// A leaf's keyword is its bare blob; a composite's names its single
    /// payload entry.
    pub const fn keyword(&self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::String => "string",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::ISize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::USize => "usize",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bytes => "bytes",
            Self::Cbor => "cbor",
            Self::RawTree => "rawtree",
            Self::Dynamic => "dynamic",
            Self::List(_) => "list",
            Self::Array { .. } => "array",
            Self::Map { .. } => "map",
            Self::Optional(_) => "optional",
            Self::Tuple(_) => "tuple",
            Self::Struct(_) => "struct",
            Self::Enum(_) => "enum",
            Self::Ref(_) => "ref",
        }
    }
}

/// Why a [`Node::Ref`] could not be resolved to a concrete node.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    /// A `ref` names a definition that is absent from `defs`.
    #[error("dangling ref {name:?}: no such definition")]
    Dangling {
        /// The missing definition name.
        name: String,
    },
    /// A chain of `ref` nodes revisited a name without reaching a concrete
    /// node, so it describes no value.
    #[error("non-productive ref cycle through {name:?}")]
    Cycle {
        /// The name the cycle was detected at.
        name: String,
    },
}

impl Schema {
    /// Build a schema document, rejecting any that is not well-formed.
    ///
    /// Runs the full well-formedness walk — dangling references, unguarded
    /// reference cycles, non-`String` map keys, oversized containers, invalid
    /// entry names, and over-deep nesting — and returns the offending
    /// [`SchemaReadError`] on the first violation. Because the fields are
    /// private, this is the only way to build a `Schema` from parts.
    pub fn new(
        kind: String,
        root: Node,
        defs: BTreeMap<String, Node>,
    ) -> Result<Self, SchemaReadError> {
        let schema = Schema { kind, root, defs };
        validate_well_formed(&schema)?;
        Ok(schema)
    }

    /// The opaque entity-kind label.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The schema of the value itself.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// The definition table for named user types.
    pub fn defs(&self) -> &BTreeMap<String, Node> {
        &self.defs
    }

    /// Validate this document and return the [`CheckedSchema`] token that the
    /// schema-directed entry points accept.
    ///
    /// Runs the well-formedness walk exactly once; the returned borrow records
    /// the fact so no entry point has to re-check.
    pub fn checked(&self) -> Result<CheckedSchema<'_>, SchemaReadError> {
        CheckedSchema::try_from(self)
    }

    /// The concrete node the [`root`](Self::root) resolves to, following `ref`
    /// nodes through [`defs`](Self::defs).
    ///
    /// A schema's root is normally a `ref` naming a defined type, so callers
    /// that care about the root's *shape* (for example, whether a document
    /// root is a struct) must resolve it rather than match [`root`](Self::root)
    /// directly.
    pub fn resolved_root(&self) -> Result<&Node, RefError> {
        self.root.resolve(&self.defs)
    }
}

/// The payload shape of one enum variant.
#[derive(Debug, Clone, PartialEq, Facet)]
#[facet(rename_all = "lowercase")]
#[repr(u8)]
pub enum VariantKind {
    /// No payload: the variant's encoding is a bare tag blob.
    Unit,
    /// A tuple variant. A single field is encoded directly; otherwise its
    /// payload is an ordinal-named tree.
    Tuple(Vec<Node>),
    /// A struct variant: a name-keyed tree, even with one field.
    Struct(BTreeMap<String, Node>),
}

/// A dynamic value's runtime kind, for mismatch diagnostics in the
/// schema-directed writer.
///
/// One home with the full arm set, so the value's kind is reported the same
/// way from every walk that can reject it.
#[cfg(feature = "value")]
pub(crate) fn value_kind(v: &Value) -> &'static str {
    if v.is_null() {
        "null"
    } else if v.is_bool() {
        "bool"
    } else if v.is_number() {
        "number"
    } else if v.is_string() {
        "string"
    } else if v.is_bytes() {
        "bytes"
    } else if v.is_array() {
        "array"
    } else if v.is_object() {
        "object"
    } else if v.is_char() {
        "char"
    } else if v.is_datetime() {
        "datetime"
    } else if v.is_qname() {
        "qname"
    } else if v.is_uuid() {
        "uuid"
    } else {
        "value"
    }
}

/// Generate the [`Schema`] describing how `T` is encoded.
///
/// The shorthand for [`Schema::from_shape`] applied to `T`'s shape.
///
/// ```
/// use facet::Facet;
/// use facet_git_tree::{Node, schema_of};
///
/// #[derive(Facet)]
/// struct Point {
///     x: f64,
///     y: f64,
/// }
///
/// let doc = schema_of::<Point>()?;
/// assert_eq!(doc.kind(), "Point");
/// assert_eq!(doc.root(), &Node::Ref("Point".into()));
/// assert!(doc.defs().contains_key("Point"));
/// # Ok::<(), facet_git_tree::SchemaError>(())
/// ```
pub fn schema_of<T: for<'a> Facet<'a>>() -> Result<Schema, SchemaError> {
    Schema::from_shape(<T as Facet>::SHAPE)
}

impl Schema {
    /// Generate the [`Schema`] describing how values of `shape` are encoded.
    ///
    /// The `kind` label is the original shape's effective name, before
    /// transparency collapse. Named user types are deduplicated into
    /// [`defs`](Schema::defs) and referenced by [`Node::Ref`]; names are
    /// assigned deterministically in pre-order. Non-string-keyed maps are
    /// represented as [`Node::Cbor`]. Oversized schema trees, duplicate
    /// effective field or variant names, and unguarded recursive shapes are
    /// rejected as [`SchemaError::UnsupportedShape`].
    pub fn from_shape(shape: &'static Shape) -> Result<Self, SchemaError> {
        Self::from_shape_with_limit(shape, MAX_VALUE_DEPTH)
    }

    /// [`from_shape`](Self::from_shape) with a custom nesting bound in place
    /// of [`MAX_VALUE_DEPTH`].
    ///
    /// Exists so tests can exercise the depth guard without a pathologically
    /// deep type (whose `SHAPE` evaluation is prohibitively expensive to
    /// compile); not part of the public API.
    #[doc(hidden)]
    pub fn from_shape_with_limit(shape: &'static Shape, limit: usize) -> Result<Self, SchemaError> {
        let mut walker = Walker::new(limit);
        let root = walker.node(shape, 0)?;
        if walker.names.len() + walker.identities > MAX_TREE_ENTRIES {
            return Err(SchemaError::UnsupportedShape(shape.type_identifier));
        }
        walker.ensure_productive()?;
        Ok(Schema {
            kind: shape.effective_name().to_owned(),
            root,
            defs: walker.defs,
        })
    }
}

/// The `Shape` → [`Node`] walker: tracks named-type definitions and the
/// deterministic name assignment while recursing through a shape.
struct Walker {
    /// Finished (or in-progress) definitions, keyed by assigned name.
    defs: BTreeMap<String, Node>,
    /// Assigned name per user type, keyed by type identity.
    names: HashMap<ConstTypeId, String>,
    /// Type identifiers for reporting invalid generated definitions.
    identifiers: HashMap<String, &'static str>,
    /// How many types have claimed each identifier, for `_2`, `_3`, …
    /// disambiguation.
    claimed: HashMap<&'static str, usize>,
    /// Number of reserved identity definitions.
    identities: usize,
    /// The nesting bound `node` enforces — [`MAX_VALUE_DEPTH`] outside of tests.
    limit: usize,
}

impl Walker {
    fn new(limit: usize) -> Self {
        Walker {
            defs: BTreeMap::new(),
            names: HashMap::new(),
            identifiers: HashMap::new(),
            claimed: HashMap::new(),
            identities: 0,
            limit,
        }
    }

    /// Reject generated definition cycles that do not cross a productive
    /// container, mapping [`scan_guards`]'s finding to the identifier of the
    /// definition whose traversal reached it.
    fn ensure_productive(&self) -> Result<(), SchemaError> {
        match scan_guards(&self.defs) {
            GuardScan::Productive => Ok(()),
            GuardScan::Dangling(_) => Err(SchemaError::UnsupportedShape("dangling schema ref")),
            GuardScan::Cyclic(cycle) => {
                let identifier = self
                    .identifiers
                    .get(cycle.root)
                    .copied()
                    .unwrap_or("recursive schema");
                Err(SchemaError::UnsupportedShape(identifier))
            }
        }
    }

    /// The schema of one shape, with `#[facet(facet_git_tree::identity_key)]`
    /// compiled into the reserved definition that marks an identity- or
    /// key-bearing subtree (see [`crate::normal_form`]).
    fn node(&mut self, shape: &'static Shape, depth: usize) -> Result<Node, SchemaError> {
        let marked = attr::is_identity_key(shape.attributes);
        let node = self.unmarked_node(shape, depth)?;
        Ok(self.mark_identity(node, marked))
    }

    /// Register `node` under a reserved [`IDENTITY_DEF_PREFIX`] definition
    /// when `marked`, returning the [`Node::Ref`] that stands in for it — a
    /// reference adds no tree level, so a marked subtree encodes exactly as
    /// an unmarked one does. Bodies are deduplicated, and an already-marked
    /// node is returned untouched, so marking is idempotent.
    ///
    /// Dedup is a linear scan of `defs`, which is fine for the handful of
    /// marked subtrees a schema carries; index by body if marked subtrees
    /// ever number in the thousands.
    fn mark_identity(&mut self, node: Node, marked: bool) -> Node {
        if !marked {
            return node;
        }
        if let Node::Ref(name) = &node
            && name.starts_with(IDENTITY_DEF_PREFIX)
        {
            return node;
        }
        if let Some((name, _)) = self
            .defs
            .iter()
            .find(|(name, body)| name.starts_with(IDENTITY_DEF_PREFIX) && **body == node)
        {
            return Node::Ref(name.clone());
        }
        let name = format!("{IDENTITY_DEF_PREFIX}{}", self.identities);
        self.identities += 1;
        self.defs.insert(name.clone(), node);
        Node::Ref(name)
    }

    /// The schema of one shape, mirroring `serialize_node`'s dispatch order.
    ///
    /// `depth` counts nesting levels against [`MAX_VALUE_DEPTH`], the same bound
    /// typed deserialization enforces on reads: a shape nested deeper than
    /// that could never be read back regardless of what schema described it,
    /// so generation is refused here rather than recursing unboundedly on a
    /// pathological (or adversarially deep) type.
    fn unmarked_node(&mut self, shape: &'static Shape, depth: usize) -> Result<Node, SchemaError> {
        if depth > self.limit {
            return Err(SchemaError::MaxDepth(self.limit));
        }

        // Transparency collapse (smart pointers and transparent newtypes),
        // mirroring `Peek::innermost_peek` on the write side: neither carries
        // information the encoding records, so neither appears in a schema.
        let shape = collapse(shape)?;

        // RawTree → a verbatim tree reference.
        if matches!(classify(shape), ShapeClass::RawTree) {
            return Ok(Node::RawTree);
        }

        // Dynamic value → shape decided at runtime, not describable further.
        if matches!(classify(shape), ShapeClass::Dynamic) {
            return Ok(Node::Dynamic);
        }

        // Unit is a marker tree, rather than a scalar text blob.
        if shape.scalar_type() == Some(ScalarType::Unit) {
            return Ok(Node::Unit);
        }

        // Scalar leaf → the per-width scalar table.
        if matches!(classify(shape), ShapeClass::Scalar) {
            return scalar_schema(shape);
        }

        // Byte sequence → a single blob, before generic sequence handling,
        // exactly as the encoder special-cases it.
        if matches!(classify(shape), ShapeClass::Bytes) {
            return Ok(Node::Bytes);
        }

        // Struct or tuple. An anonymous tuple `(A, B)` has no user name and is
        // inlined; unit structs, tuple structs, and named-field structs are
        // named user types and live in `defs`.
        if matches!(classify(shape), ShapeClass::Struct)
            && let facet::Type::User(facet::UserType::Struct(st)) = shape.ty
        {
            if st.fields.len() > MAX_TREE_ENTRIES {
                return Err(SchemaError::UnsupportedShape(shape.type_identifier));
            }
            if matches!(st.kind, facet::StructKind::Tuple) {
                return Ok(Node::Tuple(self.field_schemas(st.fields, depth + 1)?));
            }
            return self.define(shape, |walker| match st.kind {
                facet::StructKind::Unit => Ok(Node::Unit),
                facet::StructKind::TupleStruct => {
                    Ok(Node::Tuple(walker.field_schemas(st.fields, depth + 1)?))
                }
                _ => Ok(Node::Struct(walker.named_field_schemas(
                    shape,
                    st.fields,
                    depth + 1,
                )?)),
            });
        }

        // Sequences, maps, options — the same order the encoder checks them.
        match (classify(shape), shape.def) {
            (ShapeClass::Sequence, Def::List(d)) => {
                return Ok(Node::List(Box::new(self.node(d.t, depth + 1)?)));
            }
            (ShapeClass::Sequence, Def::Slice(d)) => {
                return Ok(Node::List(Box::new(self.node(d.t, depth + 1)?)));
            }
            (ShapeClass::Sequence, Def::Array(d)) => {
                if d.n > MAX_TREE_ENTRIES {
                    return Err(SchemaError::UnsupportedShape(shape.type_identifier));
                }
                return Ok(Node::Array {
                    elem: Box::new(self.node(d.t, depth + 1)?),
                    len: d.n,
                });
            }
            (ShapeClass::Map, Def::Map(d)) => {
                // Only unwrapped string keys have a native tree layout. An
                // identity-marked key cannot remain a direct String node,
                // so model the whole map as an opaque payload instead.
                if attr::is_identity_key(d.k.attributes) {
                    return Ok(Node::Cbor);
                }
                let key_shape = collapse(d.k)?;
                if !matches!(
                    key_shape.scalar_type(),
                    Some(ScalarType::Str | ScalarType::String | ScalarType::CowStr)
                ) {
                    return Ok(Node::Cbor);
                }
                return Ok(Node::Map {
                    key: Box::new(Node::String),
                    value: Box::new(self.node(d.v, depth + 1)?),
                });
            }
            (ShapeClass::Option, Def::Option(d)) => {
                return Ok(Node::Optional(Box::new(self.node(d.t, depth + 1)?)));
            }
            _ => {}
        }

        // Enum → a named user type in `defs`, with each variant's payload
        // classified exactly as the encoder classifies it.
        if matches!(classify(shape), ShapeClass::Enum)
            && let facet::Type::User(facet::UserType::Enum(et)) = shape.ty
        {
            if et.variants.len() > MAX_TREE_ENTRIES {
                return Err(SchemaError::UnsupportedShape(shape.type_identifier));
            }
            return self.define(shape, |walker| {
                let mut variants = BTreeMap::new();
                for variant in et.variants {
                    if variant.data.fields.len() > MAX_TREE_ENTRIES {
                        return Err(SchemaError::UnsupportedShape(shape.type_identifier));
                    }
                    let kind = match variant.data.kind {
                        facet::StructKind::Unit => VariantKind::Unit,
                        facet::StructKind::Tuple | facet::StructKind::TupleStruct => {
                            VariantKind::Tuple(
                                walker.field_schemas(variant.data.fields, depth + 1)?,
                            )
                        }
                        facet::StructKind::Struct => VariantKind::Struct(
                            walker.named_field_schemas(shape, variant.data.fields, depth + 1)?,
                        ),
                    };
                    if variants
                        .insert(variant.effective_name().to_owned(), kind)
                        .is_some()
                    {
                        return Err(SchemaError::UnsupportedShape(shape.type_identifier));
                    }
                }
                Ok(Node::Enum(variants))
            });
        }

        Err(SchemaError::UnsupportedShape(shape.type_identifier))
    }

    /// The schemas of positional fields, in declaration order.
    fn field_schemas(
        &mut self,
        fields: &'static [facet::Field],
        depth: usize,
    ) -> Result<Vec<Node>, SchemaError> {
        fields
            .iter()
            .map(|field| {
                let node = self.node(field.shape(), depth)?;
                Ok(self.mark_identity(node, attr::is_identity_key(field.attributes)))
            })
            .collect()
    }

    /// The schemas of struct fields, keyed by field name.
    fn named_field_schemas(
        &mut self,
        shape: &'static Shape,
        fields: &'static [facet::Field],
        depth: usize,
    ) -> Result<BTreeMap<String, Node>, SchemaError> {
        let mut nodes = BTreeMap::new();
        for field in fields {
            let node = self.node(field.shape(), depth)?;
            let node = self.mark_identity(node, attr::is_identity_key(field.attributes));
            if nodes
                .insert(field.effective_name().to_owned(), node)
                .is_some()
            {
                return Err(SchemaError::UnsupportedShape(shape.type_identifier));
            }
        }
        Ok(nodes)
    }

    /// Register `shape` as a named definition and return the [`Node::Ref`]
    /// to it, computing the body via `body` on first encounter.
    ///
    /// The name is claimed before the body is computed, so a recursive type
    /// (`struct Node { children: Vec<Node> }`) resolves its own occurrences to
    /// the already-assigned `Ref` instead of recursing forever. Distinct types
    /// sharing an identifier get `_2`, `_3`, … suffixes in pre-order, keeping
    /// name assignment deterministic.
    fn define(
        &mut self,
        shape: &'static Shape,
        body: impl FnOnce(&mut Self) -> Result<Node, SchemaError>,
    ) -> Result<Node, SchemaError> {
        if let Some(name) = self.names.get(&shape.id) {
            return Ok(Node::Ref(name.clone()));
        }
        if self.names.len() + self.identities >= MAX_TREE_ENTRIES {
            return Err(SchemaError::UnsupportedShape(shape.type_identifier));
        }
        // Suffixes can themselves be another type's original identifier.
        let claimed = self.claimed.entry(shape.type_identifier).or_insert(0);
        let name = loop {
            *claimed += 1;
            let candidate = if *claimed == 1 {
                shape.type_identifier.to_owned()
            } else {
                format!("{}_{claimed}", shape.type_identifier)
            };
            if !self.identifiers.contains_key(&candidate) {
                break candidate;
            }
        };
        self.names.insert(shape.id, name.clone());
        self.identifiers.insert(name.clone(), shape.type_identifier);
        let schema = body(self)?;
        self.defs.insert(name.clone(), schema);
        Ok(Node::Ref(name))
    }
}

/// Collapse smart pointers and transparent newtypes to the shape the encoder
/// actually writes.
///
/// Delegates the walk itself to [`collapse_shape`], the same shape-level
/// transparency collapse [`crate::de`] and [`crate::ser`]'s map-key layout
/// checks use, so all three altitudes agree on what a shape collapses to.
/// This wrapper adds what only a full schema needs: an opaque pointer shape
/// (no pointee to collapse to) is not just "not a scalar" here as it is for a
/// map-key check — a schema document has no way to describe an indescribable
/// shape at all, so it is a hard [`SchemaError::MissingPointee`].
fn collapse(shape: &'static Shape) -> Result<&'static Shape, SchemaError> {
    let shape = collapse_shape(shape);
    if let Def::Pointer(pd) = shape.def {
        // `collapse_shape` only stops on a `Def::Pointer` when it has no
        // pointee to continue through.
        debug_assert!(pd.pointee.is_none());
        return Err(SchemaError::MissingPointee(shape.type_identifier));
    }
    Ok(shape)
}

/// The authoritative supported-scalar table, mirroring [`facet::ScalarType`].
///
/// Every textual scalar (`str`, `String`, `Cow<str>`) maps to [`Node::String`];
/// every numeric width maps 1:1. Unit is handled separately as a marker tree;
/// other scalars outside the table are unsupported.
static SUPPORTED_SCALARS: &[(ScalarType, Node)] = &[
    (ScalarType::Bool, Node::Bool),
    (ScalarType::Char, Node::Char),
    (ScalarType::Str, Node::String),
    (ScalarType::String, Node::String),
    (ScalarType::CowStr, Node::String),
    (ScalarType::F32, Node::F32),
    (ScalarType::F64, Node::F64),
    (ScalarType::U8, Node::U8),
    (ScalarType::U16, Node::U16),
    (ScalarType::U32, Node::U32),
    (ScalarType::U64, Node::U64),
    (ScalarType::U128, Node::U128),
    (ScalarType::USize, Node::USize),
    (ScalarType::I8, Node::I8),
    (ScalarType::I16, Node::I16),
    (ScalarType::I32, Node::I32),
    (ScalarType::I64, Node::I64),
    (ScalarType::I128, Node::I128),
    (ScalarType::ISize, Node::ISize),
];

pub(crate) fn scalar_node(scalar: ScalarType) -> Option<Node> {
    SUPPORTED_SCALARS
        .iter()
        .find(|(supported, _)| *supported == scalar)
        .map(|(_, node)| node.clone())
}

fn scalar_schema(shape: &'static Shape) -> Result<Node, SchemaError> {
    let Some(scalar) = shape.scalar_type() else {
        return Err(SchemaError::UnsupportedScalar(shape.type_identifier));
    };
    scalar_node(scalar).ok_or(SchemaError::UnsupportedScalar(shape.type_identifier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_scalar_table_is_exhaustive_and_self_consistent() {
        assert_eq!(SUPPORTED_SCALARS.len(), 19);
        for (scalar, expected) in SUPPORTED_SCALARS {
            assert_eq!(scalar_node(*scalar), Some(expected.clone()));
        }
        assert!(scalar_node(ScalarType::Unit).is_none());
        assert!(scalar_node(ScalarType::ConstTypeId).is_none());
    }
}
