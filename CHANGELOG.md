# Changelog

## [0.2.0-alpha.3](https://github.com/git-ents/facet-git-tree/compare/facet-git-tree-v0.1.0-alpha.3...facet-git-tree-v0.2.0-alpha.3) (2026-11-11)


### Features

* Add a dynamic RawBlob schema node ([6eeea4f](https://github.com/git-ents/facet-git-tree/commit/6eeea4f5b7fcc0f8ab90fb4c4463cae3e8292cba))
* Add the .schema document envelope ([47ab29a](https://github.com/git-ents/facet-git-tree/commit/47ab29a7c824cd0e067d953eac8c915988d675ef))
* Add the schema-of-schemas fixed-point check ([9347d59](https://github.com/git-ents/facet-git-tree/commit/9347d59ea7f4bcf89df97458cf6e1e5783d6eaf5))
* Add typed document storage ([ef7d52c](https://github.com/git-ents/facet-git-tree/commit/ef7d52c52353e9f4bd1d2c55704c8695d6565a16))
* Implement the frozen git-tree wire format ([cdc1989](https://github.com/git-ents/facet-git-tree/commit/cdc19891f0e71af5ed2abc34bbdde7d6e9bc8f27))


### Bug Fixes

* **de:** Reconcile the tree entry pre-scan count with the decoded entries ([d59852d](https://github.com/git-ents/facet-git-tree/commit/d59852d54ca7c6c843ba5e2ef5f9bb8cec6f7b5e))
* Keep CBOR validation lint-clean without the value feature ([a3b4bd9](https://github.com/git-ents/facet-git-tree/commit/a3b4bd90916c2a65f8ae7dc3194c0297f2960c7c))


### Documentation

* Add the wire-format implementation dev plan ([9a39890](https://github.com/git-ents/facet-git-tree/commit/9a398909bfc4e6ca0845dd1d453dbdd281779397))
* Correct the escaping example for a non-ASCII name ([a86b58b](https://github.com/git-ents/facet-git-tree/commit/a86b58bda80839cb71f45e30581c8ebfc602a684))
* Describe the document envelope in the readme ([98c943e](https://github.com/git-ents/facet-git-tree/commit/98c943e4ebbfdb7e33edf5e7414f4dbaebd3ee7e))
* Describe the typed object ids and checked schemas ([25c5c59](https://github.com/git-ents/facet-git-tree/commit/25c5c597f34ef8c239560306276926ac7c296346))
* Document the frozen wire format ([1484a88](https://github.com/git-ents/facet-git-tree/commit/1484a88e392e83df99c305093f4efbb8060b72ea))
* Document the RawBlob schema node ([1599a11](https://github.com/git-ents/facet-git-tree/commit/1599a119771612a9ee304ac1fcca6ae7c5d42ed4))
* Drop pre-freeze names from the shared fixtures ([0d07456](https://github.com/git-ents/facet-git-tree/commit/0d07456dfdc3e470f2c05953ae476927cf8abe4f))
* Freeze the git-tree wire format ([1c10368](https://github.com/git-ents/facet-git-tree/commit/1c10368d34fd248f55dda962a43ed5a8e24dcdce))
* Record the `.schema` envelope ownership issue ([250dd9d](https://github.com/git-ents/facet-git-tree/commit/250dd9d954238d20dbe8f01f793f63ec76a89d40))
* Require a struct document root ([7dad00e](https://github.com/git-ents/facet-git-tree/commit/7dad00e4224935f157f0f415c5e2e02ed82f3a10))
* Store non-finite floats instead of rejecting them ([cfed1d5](https://github.com/git-ents/facet-git-tree/commit/cfed1d5ee8c0bbc8a2f21af4790b481ac74a3847))


### Code Refactoring

* Apply code simplifications ([f15a790](https://github.com/git-ents/facet-git-tree/commit/f15a7909ab8b044a59a3b17705fcd7f7626d25dd))
* Carry a validated schema through the schema-directed entry points ([5422dda](https://github.com/git-ents/facet-git-tree/commit/5422dda6d22e4f4d27cac93f323ef286dc18517f))
* Carry validated tree entries through the decode path ([8b10146](https://github.com/git-ents/facet-git-tree/commit/8b10146bb37174a3b23a9d9b5e757cbb66457ebb))
* Centralize schema ref resolution ([458547b](https://github.com/git-ents/facet-git-tree/commit/458547b4ffb6f408dc97b022187c76208b9ff56c))
* Give normal-form limits and map keys representable types ([466d92d](https://github.com/git-ents/facet-git-tree/commit/466d92da5e97e20cfd9c4abbb64177c3c1645301))
* Parameterize object ids by hash format and migrate tests ([9a925d7](https://github.com/git-ents/facet-git-tree/commit/9a925d7b1323e9244d29ffde2e768d81995a4b4d))


### Tests

* Compile the README example as a doctest ([93837b4](https://github.com/git-ents/facet-git-tree/commit/93837b4910a588c06fa3474edd52d5d364e5cfd1))

## [0.1.0-alpha.3](https://github.com/git-ents/facet-git-tree/compare/facet-git-tree-v0.1.0-alpha.1...facet-git-tree-v0.1.0-alpha.3) (2026-10-22)

Note that `v0.1.0-alpha.2` was published accidentally, and was therefore yanked.
That yanked pre-release did not contain substantive changes.

### ⚠ BREAKING CHANGES

* Remove the migration, schema-pinning, and schema-kind APIs; schema documents no longer carry `kind`, so previously published schema object ids change and documents written in the old shape no longer read

### chore

* Pin release ([cfdbd1c](https://github.com/git-ents/facet-git-tree/commit/cfdbd1c814b88865117ca899b77a89dbe57ebd65))
* Pin second alpha release ([435a9bb](https://github.com/git-ents/facet-git-tree/commit/435a9bb5aff1691fbd483767d040265fd3cc0434))


### Bug Fixes

* Schema documents no longer carry `kind`, so previously published schema object ids change and documents written in the old shape no longer read ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))


### Documentation

* Convert the code of conduct to Markdown ([e3f05a8](https://github.com/git-ents/facet-git-tree/commit/e3f05a8991ba1f2d7869a4cf6576598e65c7537e))
* Convert the contribution guide to Markdown ([b8fc786](https://github.com/git-ents/facet-git-tree/commit/b8fc786dc5c57f95c136d221145d676add29f874))
* Convert the README to Markdown ([19dbb2b](https://github.com/git-ents/facet-git-tree/commit/19dbb2bce08c75fdf6908be508012b1940e066b2))
* Fix the broken contribution guide reference in the README ([a2e6581](https://github.com/git-ents/facet-git-tree/commit/a2e6581964f69cf4a3c1c669629d14aeb265211c))
* Point the crate manifest readme field at README.md ([19dbb2b](https://github.com/git-ents/facet-git-tree/commit/19dbb2bce08c75fdf6908be508012b1940e066b2))
* Update README with TODO content ([dbbf548](https://github.com/git-ents/facet-git-tree/commit/dbbf548739e8e1328c050c20ea178c68416adcc8))
* Update the code of conduct references in README.adoc and CONTRIBUTING.adoc ([e3f05a8](https://github.com/git-ents/facet-git-tree/commit/e3f05a8991ba1f2d7869a4cf6576598e65c7537e))
* Update the contribution guide reference in the README ([b8fc786](https://github.com/git-ents/facet-git-tree/commit/b8fc786dc5c57f95c136d221145d676add29f874))


### Code Refactoring

* `Schema::from_shape_with_limit` now returns `Result<Schema, SchemaError>` ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Collapse the field-node traits into one ([7abeded](https://github.com/git-ents/facet-git-tree/commit/7abeded8bc35253501724bb9771140a8d8cba6a0))
* Drop dead code, redundant tests, and extra examples ([1f6136f](https://github.com/git-ents/facet-git-tree/commit/1f6136f59bf4b358f82fe2dcacd15b952f68f352))
* Remove `Attr::RenamedFrom` ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `DecodeMode` and the legacy leaf decode mode ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `deserialize_into` ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `deserialize_legacy_leaves` and the schema-driven `deserialize_value_with_schema_legacy_leaves` ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `EMPTY_TREE` and `SchemaSchema` ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `Hints`, `schema_and_hints_of`, and `from_shape_with_hints` ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `Schema::kind` and its publication policy (`ANONYMOUS_KIND`, `with_kind`, `set_kind`, `validate`, `validate_publishable`) ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove `SchemaPinError` and its variants ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove schema pinning and legacy compatibility ([2408df8](https://github.com/git-ents/facet-git-tree/commit/2408df827a95df6cd6243b51ba4424c47c097e57))
* Remove Schema::kind and its publication policy ([01ad002](https://github.com/git-ents/facet-git-tree/commit/01ad0029bbfdfedd9cbc5bee5449336e75e63c97))
* Remove the `migration` module (`Migration`, `MigrationSchema`, `MigrationError`, `MigrationPinError`, `Op`, `Target`, `Change`, `Derivation`) ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))
* Remove the migration subsystem ([7b5d08e](https://github.com/git-ents/facet-git-tree/commit/7b5d08e982de0f3818f9622492cf303b45b8759a))
* Simplify the crate by deleting migration, pinning, and kind ([3d41507](https://github.com/git-ents/facet-git-tree/commit/3d41507aca33d8b0b0ee4f3f1127912c3ad87b14))


### Tests

* Cover the typed map key check through serialize ([54e992a](https://github.com/git-ents/facet-git-tree/commit/54e992a2b2bbfab5bac75af06f8a3d8d6f112836))
* Pin the plain-tree shape of a struct ([d59a57a](https://github.com/git-ents/facet-git-tree/commit/d59a57a0391f3f9180b209d07ce92c6e507ffd9c))

## 0.1.0-alpha.1 (2026-10-10)

### Features

- Add encoding for core types ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))
- Add support for dynamic values ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))

### Tests

- Add initial unit tests ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))
- Add initial integration tests ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))

### Dependencies

- Update facet dependencies ([73726b8](https://github.com/git-ents/facet-git-tree/commit/73726b88d0bdfe02e53f4f5ccaef6f4f3689d395))
