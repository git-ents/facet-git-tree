# `facet-git-tree`

*[facet.rs](https://facet.rs) format crate for Git objects.*

`facet-git-tree` encodes `Facet` values as content-addressed Git blobs and trees and decodes them using their Rust type or a stored schema.
The normative wire format is [`docs/format.adoc`](docs/format.adoc); it supersedes older descriptions in the `git-store` specification.
The format is stable within a major version, but is pre-alpha and is not compatible with the previous layout.

## Typed values

Add `facet-git-tree` and `facet` to your project, derive `Facet`, and use the bare-value codec:

```rust
use facet::Facet;
use facet_git_tree::{deserialize, serialize};

#[derive(Debug, PartialEq, Facet)]
struct Book {
    title: String,
    pages: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let book = Book { title: "Git Trees".into(), pages: 120 };
    let (oid, store) = serialize(&book)?;
    let decoded: Book = deserialize(&oid, &store)?;
    assert_eq!(decoded, book);
    Ok(())
}
```

`serialize` returns the root object ID and a fresh in-memory `ObjectStore`.
To use an existing store, call `serialize_into(&value, &store)`; it accepts a format-tagged `gix_object::Write` sink.
`deserialize(&oid, &store)` accepts a format-tagged `gix_object::Find` source, including a suitable `gix` object database.
`ObjectStore` defaults to SHA-1; `ObjectStore::<Sha256>::new()` selects SHA-256, and `Backend::<F, _>::new(odb)` tags an arbitrary backend with its format.
Object IDs are typed: an `Oid<F>` fixes its Git hash format in the type, so an id and a store of different formats cannot be paired.
Object IDs are comparable only within the same hash format.

These APIs write and read **bare values**, not bound documents: a root can be a blob or a tree, and no `.schema` or `.value` envelope is added.
They do not publish refs or commits.
`RawTree::new(oid)` and `RawBlob::new(oid)` reference already-stored objects by ID without re-encoding them; the id's `Oid<F>` type ties it to the store's hash format, so a mismatched format is a compile error.

`check_key(&str)` checks whether a user name can be encoded within the entry-name limit.
The `normal_form` module provides `hash`/`hash_into` for the restricted identity representation; `check_universe`, `identity_subtrees`, and `check_identity_subtrees` inspect its schema constraints.
`#[facet(facet_git_tree::identity_key)]` marks a field or type as identity-bearing.

## Schemas and dynamic values

`schema_of::<T>()` (or `Schema::from_shape`) derives a self-hosted `Schema`.
`kind()` is an opaque label; named definitions are referenced by `Node::Ref`.
A `Schema` is well-formed by construction: build one from a derived shape, or from parts with the fallible `Schema::new(kind, root, defs)`, which validates every ref, key, name, and bound.
Derive an entity schema when authoring it, then store and reuse that exact schema rather than re-deriving it for each read or write.
A schema is itself a `Facet` value and can be serialized as a bare value.

Enable the `value` feature to use the schema-directed APIs with `facet_value::Value`.
A schema-directed read or write takes a `CheckedSchema`, obtained from a validated `Schema` via `Schema::checked()`, so validity is a property of the argument rather than a re-checked precondition:

- `deserialize_value_with_schema(&oid, &checked_schema, &store)` reads a bare value without its original Rust type.
- `validate_with_schema(&oid, &checked_schema, &store)` checks a value against the schema.
- `serialize_value_with_schema(&value, &checked_schema, &store)` writes a bare value, validating it against the schema; mismatches report a path.

The schema-directed and typed encodings are intended to produce the same objects for equivalent representable values.
An unwrapped `Value::NULL` represents `None`, so the value API cannot distinguish it from `Some(None)` or `Some(())`; the schema-directed reader rejects such ambiguous `Some` values.
A `Node::Dynamic` value has no on-disk kind tag and its heuristic read is lossy; it is not a substitute for a schema when type fidelity matters.
Canonical CBOR is specified by the format; implementation support is evolving, so check the crate's API documentation for current limitations.

## Documents

A *document* binds a value to the unbound schema it conforms to, so it can be read back without the original Rust type.
It is the value's own tree entries beside one reserved `.schema` entry:

```text
{ <value's own entries>..., .schema -> <unbound schema> }
```

The `document` module owns this envelope:

- `split(root, objects)` validates a document root and returns it with its embedded schema tree.
- `ensure_struct_root(schema)` refuses a schema whose resolved root is not a struct, since only a struct has named fields to sit beside `.schema`.
- `UnboundValue::new(root, objects)` presents a document root to the bare-value codec with `.schema` hidden; `read_value` wires that view to a schema-directed read.

The envelope is a pure tree-in, tree-out codec: it reads no refs and writes no commits.
Binding values under refs, commit trailers, and schema publication are the store layer's job.

## Wire format at a glance

A bound document is a tree containing its reachable schema in `.schema`.
A document's schema root must resolve to a struct, so its fields sit directly at the document root alongside `.schema`; a schema describing any other shape is a bare value, not a document.
The schema subtree is a **bare** `Schema`, not another bound document.
Binding, commits and schema publication are higher-layer operations, not the bare codec APIs above.
The commit's `Schema-Version:` identifies the format major; a bare tree has no version metadata, so cross-major interpretation must be refused.
`Schema-commit:` is advisory; `.schema` is authoritative and reachable.

- User field, variant, map-key and definition names use canonical percent escaping.
  For example, `_` stays `_`, `none` becomes `%none`, and `a/b` becomes `%a%2Fb`.
  Encoded names are limited to 255 bytes.
- Text leaves have one newline appended; typed byte sequences and canonical CBOR blobs are stored verbatim without an appended newline.
  Floats use canonical `ryu` text, including `nan`, `inf`, and `-inf`; zero signs and NaN payloads are normalized.
- Positional entries use three lowercase hexadecimal digits (`000`–`fff`), with at most 4096 entries per tree.
  Maps have string keys; other keys require a CBOR representation.
- `None`, unit and empty composites use `{ none -> <empty blob> }`; `Some(x)` uses `{ some -> x }`.
  Structs write every field, and readers reject missing or unknown fields.
  Enum variants are externally tagged: a unit variant is a text blob, and a one-field tuple variant stores its field directly.
- Readers must reject non-canonical or malformed input, including bad names, ordinals, modes, scalar encodings and schemas.
  The format limits nesting to 32 and decode visits to 2^20.
  A `RawTree` is opaque and exempt from the ordinary tree-content rules.

See the format document for the complete rules, including the lossy dynamic-value encoding and its exceptions.
Implementation work is ongoing; the specification is not a claim that every feature is already available in every build.

## Code of Conduct

Please refer to the in-source [code of conduct](/CONDUCT.md) for all behavioral expectations.

## Contribution Guide

Contributions are welcome.
Please refer to the in-source [contribution guide](/CONTRIBUTING.md).
