# Issues

## `.schema` envelope ownership: facet-git-tree defines it, git-store implements it

The document envelope is normatively defined in this crate's `docs/format.adoc` but implemented in `git-store`.
The two repositories assert opposite ownership, so any future change to the envelope — most immediately, non-struct document roots — has no clear home.

### The contradiction

`docs/format.adoc` claims the envelope as frozen format of *this* crate:

> The *schema-of-schemas* — the `Node`/`Schema` vocabulary, the `.schema` envelope, the `none`/`some` marker names, the ordinal scheme, and the `%` escape defined here — is the long-term-support artifact, versioned by a single major number.
> **Changing any rule in this document is a major-version event.**

Its `== Envelope` section then defines the whole construct: a document is `{ <value's own entries>..., .schema -> <unbound schema> }`, `.schema` is reserved only at the root, and the root MUST resolve to a struct.

`src/lib.rs` disclaims it:

> This crate writes and reads **bare values only**; the `.schema` document envelope is a `git-store` concern.

`docs/dev-plan.adoc` W7 assigns it to git-store (`git-store/crates/git-store/src/{store,identity,document}.rs`), and `git-store`'s `docs/specification.adoc` lists "the document envelope, schema publication, refs, commits, tombstones" as its own requirements.

### Why it belongs here

The implementation needs no store concepts.
`git-store`'s `src/envelope.rs` (`split`, `ensure_struct_root`, `UnboundValue`, `read_value`, `SCHEMA_ENTRY`) is generic over `gix::objs::Find` plus `Node`/`Schema`/`ObjectId` — no refs, no commits, no kinds, no tombstones.
It answers exactly "given a tree, is it a document, and read its value," which is a codec question.

The rules the envelope depends on already live here: `src/name.rs` and `src/ordinal.rs` hardcode `.schema`/`.value`/`none`/`some` for escaping and ordering.
Keeping the envelope in `git-store` splits those coupled rules across two crates — the strain that invites drift.

Moving it up makes the ownership in `format.adoc` true, and lets any consumer that can read a `facet-git-tree` value read a document without depending on `git-store`'s ref machinery.

### Counter-argument

This crate was deliberately kept value-only (see `src/lib.rs`, `dev-plan.adoc` W7).
The "root resolves to a struct" rule can be read as store *policy* — only struct-rooted documents are addressable — rather than an encoding rule.
Under that reading the current split is correct, and the fix is to stop claiming the envelope in `format.adoc` instead of moving code.

### Consequence for non-struct roots

Whichever way this resolves, it decides where non-struct document roots live.
Today the struct-root rule is in `format.adoc`, so allowing a non-struct root (which would require writing the reserved `.value` entry) reads as a `facet-git-tree` format decision — a major version.
If the envelope moves here, that framing becomes correct rather than accidental.

### Proposal

Move the envelope up:

1. Add a `document` module to this crate with `SCHEMA_ENTRY`, `split`,
   `ensure_struct_root`, `UnboundValue`, and `read_value`.
2. Relocate the error variants the envelope reports (`NotSubtreeBound`,
   `MissingObject`, `NotATree`, `SubtreeMissing`, `DocumentRootNotStruct`,
   `SchemaRootUnresolved`) into this crate's error type.
3. Reduce `git-store`'s `envelope.rs` to re-exports; the store keeps refs,
   placement, and policy.

### Open decisions

- Whether this crate's scope becomes "values + documents," or stays "values
  only" and `format.adoc` narrows to match.
- If it moves here, whether the struct-root rule remains a hard format rule or
  becomes a documented policy the store may override.
