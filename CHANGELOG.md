# Changelog

## [0.1.0-alpha.3](https://github.com/git-ents/facet-git-tree/compare/facet-git-tree-v0.1.0-alpha.1...facet-git-tree-v0.1.0-alpha.3) (2026-10-22)


### ⚠ BREAKING CHANGES

* remove the migration, schema-pinning, and schema-kind APIs; schema documents no longer carry `kind`, so previously published schema object ids change and documents written in the old shape no longer read

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
