# Changelog

## 0.1.0-alpha.1 (2026-10-10)


### ⚠ BREAKING CHANGES

* convert the documentation set from Markdown to AsciiDoc
* carry field-level default presence into the wire Schema
* pin each generation's tree to a codec fixture, not just Schema's shape
* SchemaSchema::GENESIS and MigrationSchema::GENESIS object ids changed; documents pinned to the old genesis trees are no longer recognized
* facet_git_tree::Schema and facet_git_tree::SchemaDoc swap meanings; SchemaDoc no longer exists.
* name-key Schema::Struct and Schema::Enum by field/variant name
* pin the schema-schema tree a stored schema was written against
* name-key schema struct/enum nodes and pin the schema-schema
* encode Git trees as schemaless, type-driven values
* remove `.schema` schema object and `.variant` sentinel
* drop `Error::ReservedKey` and reserved-name checks in `check_key`

### chore

* Pin initial alpha release ([9dd72d7](https://github.com/git-ents/facet-git-tree/commit/9dd72d785f09fe3203ff640efee71a0f3004ff4a))
* Pin initial alpha release ([96cd5c1](https://github.com/git-ents/facet-git-tree/commit/96cd5c1efc0a5a7400b79a87056a8db7b363fa8a))
* Pin initial alpha release ([3447605](https://github.com/git-ents/facet-git-tree/commit/3447605fdd194cbd78c19dae6182d212d81c8971))
* Pin initial release candidate ([6a06aff](https://github.com/git-ents/facet-git-tree/commit/6a06aff7667f6bada48c513e1426bf15f63d280a))
* Pin initial version ([09b09a6](https://github.com/git-ents/facet-git-tree/commit/09b09a6491841a2267f667e11a3a58139c4c9704))


### Dependencies

* Bump gix-object and gix-odb to 0.62 and 0.82 ([f95816f](https://github.com/git-ents/facet-git-tree/commit/f95816f46abdcb4dd3d9bce2190e673008bfa57f))
* Bump gix-object to 0.62.0 ([f95816f](https://github.com/git-ents/facet-git-tree/commit/f95816f46abdcb4dd3d9bce2190e673008bfa57f))
* Bump gix-odb to 0.82.0 ([f95816f](https://github.com/git-ents/facet-git-tree/commit/f95816f46abdcb4dd3d9bce2190e673008bfa57f))
* Update facet dependencies ([73726b8](https://github.com/git-ents/facet-git-tree/commit/73726b88d0bdfe02e53f4f5ccaef6f4f3689d395))


### Features

* Add `build_cache` example proving artifact dedup via object count ([1d41124](https://github.com/git-ents/facet-git-tree/commit/1d41124d6b40d67cbdc8ee61f659fc3f2adddb2a))
* Add `code_review` example dumping the nested object graph ([1d41124](https://github.com/git-ents/facet-git-tree/commit/1d41124d6b40d67cbdc8ee61f659fc3f2adddb2a))
* Add `Error::InvalidOrdinal` for non-numeric sequence entry names ([d65b873](https://github.com/git-ents/facet-git-tree/commit/d65b8736bb6c0f96ad82ed2eaf745ba07dd2a8d1))
* Add `Error::MaxDepth` bounding deserialization recursion depth ([d65b873](https://github.com/git-ents/facet-git-tree/commit/d65b8736bb6c0f96ad82ed2eaf745ba07dd2a8d1))
* Add `issues` example showing content-addressed stable root OIDs ([1d41124](https://github.com/git-ents/facet-git-tree/commit/1d41124d6b40d67cbdc8ee61f659fc3f2adddb2a))
* Add check_key and Error for tree-entry name validation ([1aebbbc](https://github.com/git-ents/facet-git-tree/commit/1aebbbcf08b642d50bb52a05917fd79b1ecf6659))
* Add dynamic Value support and self-hosted schemas ([9448ccc](https://github.com/git-ents/facet-git-tree/commit/9448ccc9710791cf2469e247e001bbf50d70b7ab))
* Add encoding for core types ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))
* Add ObjectStore in-memory gix object backend ([1aebbbc](https://github.com/git-ents/facet-git-tree/commit/1aebbbcf08b642d50bb52a05917fd79b1ecf6659))
* Add RawTree for embedding pre-written subtrees verbatim ([0efbd3f](https://github.com/git-ents/facet-git-tree/commit/0efbd3fb036095c9e05953451fb61dc5c1ae7ae4))
* Add schema migration lenses, derivation, and read-time upcast ([aa98fcb](https://github.com/git-ents/facet-git-tree/commit/aa98fcbf811328a42c924ff0016695a26720839c))
* Add schema-directed serialization ([1265118](https://github.com/git-ents/facet-git-tree/commit/1265118699111aa7e296d43cca48cf7926ed3fcc))
* Add SchemaDoc version marker for bootstrap/upgrade (d4f8aaaf) ([2329baa](https://github.com/git-ents/facet-git-tree/commit/2329baa36e1d805396895bb769bc349f6a83c6ef))
* Add serialize_value_with_schema ([1265118](https://github.com/git-ents/facet-git-tree/commit/1265118699111aa7e296d43cca48cf7926ed3fcc))
* Add serialize/serialize_into/deserialize entry points ([1aebbbc](https://github.com/git-ents/facet-git-tree/commit/1aebbbcf08b642d50bb52a05917fd79b1ecf6659))
* Add support for dynamic values ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))
* Add the identity normal form and its type-universe check ([8e72c7b](https://github.com/git-ents/facet-git-tree/commit/8e72c7ba4f58f2d5fb8706d2c463ad9119444e95))
* Align store documents and plumbing ([f94bd59](https://github.com/git-ents/facet-git-tree/commit/f94bd5904180b8928ddca85c7ed3f57c9798a822))
* Bound serialization recursion depth ([0c1fbc4](https://github.com/git-ents/facet-git-tree/commit/0c1fbc4a400576963fdd587070e7361a6f7c9f26))
* Carry field-level default presence into the wire Schema ([fdfdf04](https://github.com/git-ents/facet-git-tree/commit/fdfdf0456a776f2bfca20fc20f9fd83a4c5e1cce))
* Deserialize scalars via parse_from_str, structs via begin_field ([412941b](https://github.com/git-ents/facet-git-tree/commit/412941bb4d889cc5a031385b9463e830a58c6134))
* Encode composite-keyed maps as ordinal-named `{ k, v }` pair sub-trees ([fd59a0d](https://github.com/git-ents/facet-git-tree/commit/fd59a0d7bc90a82b028058ee577ce14c899ce57b))
* Encode scalar-keyed maps by textual key form for any scalar key type ([fd59a0d](https://github.com/git-ents/facet-git-tree/commit/fd59a0d7bc90a82b028058ee577ce14c899ce57b))
* Encode u8 sequences (Vec&lt;u8&gt;, [u8; N], slices) as a single blob ([849666a](https://github.com/git-ents/facet-git-tree/commit/849666a0c682884a78b945894216c57d24db84e0))
* **facet-git-tree:** Add raw blob passthrough ([65e74b6](https://github.com/git-ents/facet-git-tree/commit/65e74b6713fd9dd49d890163bd7a024fae3472d6))
* Harden deserialization against malformed foreign trees ([d65b873](https://github.com/git-ents/facet-git-tree/commit/d65b8736bb6c0f96ad82ed2eaf745ba07dd2a8d1))
* Implement serialize and deserialize for scalars and composites ([412941b](https://github.com/git-ents/facet-git-tree/commit/412941bb4d889cc5a031385b9463e830a58c6134))
* Land the wave 1 format break and promote the git-store pitch ([b1b2aa6](https://github.com/git-ents/facet-git-tree/commit/b1b2aa68dd1414f7a553598b0168b588b9271d9b))
* Name composite-key map pair entries by their pair tree's object id ([c77aea0](https://github.com/git-ents/facet-git-tree/commit/c77aea0b372dab3b508fb3173d7c0d949294c18e))
* Name-key schema struct/enum nodes and pin the schema-schema ([df79233](https://github.com/git-ents/facet-git-tree/commit/df792331e322a403cfbae1594c1d8bd4119a2801))
* Name-key Schema::Struct and Schema::Enum by field/variant name ([df79233](https://github.com/git-ents/facet-git-tree/commit/df792331e322a403cfbae1594c1d8bd4119a2801))
* Normalize NaN to "nan" and negative zero to positive zero on write ([412941b](https://github.com/git-ents/facet-git-tree/commit/412941bb4d889cc5a031385b9463e830a58c6134))
* Pin the schema-schema tree a stored schema was written against ([df79233](https://github.com/git-ents/facet-git-tree/commit/df792331e322a403cfbae1594c1d8bd4119a2801))
* Refuse a stored schema version of 0, numbering starting at 1 ([2329baa](https://github.com/git-ents/facet-git-tree/commit/2329baa36e1d805396895bb769bc349f6a83c6ef))
* Refuse schemas whose identity subtree leaves the normal form ([5b4c6be](https://github.com/git-ents/facet-git-tree/commit/5b4c6becb09a5c9ab62134e524e8727941670c50))
* Refuse to publish over a schema tip whose version is unreadable ([2329baa](https://github.com/git-ents/facet-git-tree/commit/2329baa36e1d805396895bb769bc349f6a83c6ef))
* Scaffold facet-git-tree serialization API and encoding spec ([1aebbbc](https://github.com/git-ents/facet-git-tree/commit/1aebbbcf08b642d50bb52a05917fd79b1ecf6659))
* Serialize and deserialize smart pointers (Box/Arc/Rc, including Arc&lt;[T]&gt;) ([849666a](https://github.com/git-ents/facet-git-tree/commit/849666a0c682884a78b945894216c57d24db84e0))
* Serialize scalars to UTF-8 blobs ([412941b](https://github.com/git-ents/facet-git-tree/commit/412941bb4d889cc5a031385b9463e830a58c6134))
* Serialize structs and tuples as Git trees ([412941b](https://github.com/git-ents/facet-git-tree/commit/412941bb4d889cc5a031385b9463e830a58c6134))
* Splice a shared {schema/, value/} codec fixture onto every schema-schema and migration-schema generation tree ([684e173](https://github.com/git-ents/facet-git-tree/commit/684e17336802f5390bd54d701bc5e5da33cea588))
* Store byte sequences as blobs and support smart pointers ([849666a](https://github.com/git-ents/facet-git-tree/commit/849666a0c682884a78b945894216c57d24db84e0))
* Support non-string and composite map keys ([fd59a0d](https://github.com/git-ents/facet-git-tree/commit/fd59a0d7bc90a82b028058ee577ce14c899ce57b))
* Support transparent newtypes in serialization and deserialization ([57c6ccc](https://github.com/git-ents/facet-git-tree/commit/57c6ccc2182849a4f5120df77dd690a531b49a06))
* Surface NotFound, NotATree, NonUtf8Name on malformed input ([412941b](https://github.com/git-ents/facet-git-tree/commit/412941bb4d889cc5a031385b9463e830a58c6134))


### Bug Fixes

* Access Attr fields through ns()/key() methods ([e02a08d](https://github.com/git-ents/facet-git-tree/commit/e02a08d6128e74f31be8c67b77fcd1d944486c28))
* Append mandatory trailing newline to every leaf blob (5b39f084) ([f5fdb13](https://github.com/git-ents/facet-git-tree/commit/f5fdb1393784d7da8ecdfc49717cbc50b3d6a810))
* Apply review fixes to the codec and schema layers ([22e279f](https://github.com/git-ents/facet-git-tree/commit/22e279f699a3c5912c196a149f3c343cca698d25))
* Check NormalForm::Struct field names against the shared tree-entry rules ([98a1c39](https://github.com/git-ents/facet-git-tree/commit/98a1c390740134dbe13034ebf0ea8921d4406f6d))
* Correct behaviour -&gt; behavior typo in a gix-store test doc comment ([2329baa](https://github.com/git-ents/facet-git-tree/commit/2329baa36e1d805396895bb769bc349f6a83c6ef))
* Correct the normal-form list length bound and pin the ordinal boundary ([4b64199](https://github.com/git-ents/facet-git-tree/commit/4b64199539160e7244a3c6e403f05c13d105f89b))
* Detect tuple vs struct variant fields by numeric field name ([6517e44](https://github.com/git-ents/facet-git-tree/commit/6517e44bebbc193502814d09820564abdbb25689))
* Distinguish corrupt from missing objects in ObjectStore::get ([2592843](https://github.com/git-ents/facet-git-tree/commit/2592843f89313e471d8551349c5c0b4a57d67629))
* Enforce canonical git's full tree-entry-name rules via one shared validator ([6520ce9](https://github.com/git-ents/facet-git-tree/commit/6520ce917581482f55c275d267a267f7c769830b))
* Lower MAX_DEPTH to 32 so the recursion guard fires before exhausting a 2 MiB thread stack ([b28f29d](https://github.com/git-ents/facet-git-tree/commit/b28f29dcf239092ffa9b25ca10ac7685350cb677))
* Normalize release pull request titles ([6870291](https://github.com/git-ents/facet-git-tree/commit/68702913ee548c72eb434d662a374dd593048731))
* Pin each generation's tree to a codec fixture, not just Schema's shape ([684e173](https://github.com/git-ents/facet-git-tree/commit/684e17336802f5390bd54d701bc5e5da33cea588))
* Place array elements by parsed ordinal rather than iteration index ([b28f29d](https://github.com/git-ents/facet-git-tree/commit/b28f29dcf239092ffa9b25ca10ac7685350cb677))
* Prevent deserialization stack overflow and harden malformed trees ([b28f29d](https://github.com/git-ents/facet-git-tree/commit/b28f29dcf239092ffa9b25ca10ac7685350cb677))
* Refuse the anonymous kind sentinel at the publication boundary ([3a7a8d0](https://github.com/git-ents/facet-git-tree/commit/3a7a8d0fc79843a7469410e29b90507a1abfbddd))
* Reject a struct write that omits a schema-required field ([8338699](https://github.com/git-ents/facet-git-tree/commit/83386993d17ba1a9f01b82183ba371671fbf475f))
* Reject enum trees that do not have exactly one entry ([b28f29d](https://github.com/git-ents/facet-git-tree/commit/b28f29dcf239092ffa9b25ca10ac7685350cb677))
* Reject NUL in dynamic keys with KeyError, not an opaque backend error ([227b945](https://github.com/git-ents/facet-git-tree/commit/227b945cecaed77f7e2cc2c982ad9478e2105d61))
* Reject Option trees that are neither empty nor a single "some" entry ([d65b873](https://github.com/git-ents/facet-git-tree/commit/d65b8736bb6c0f96ad82ed2eaf745ba07dd2a8d1))
* Remove dead FieldNode default method ([99939d8](https://github.com/git-ents/facet-git-tree/commit/99939d86a3dae23b598cc531a684dccb02a9ec54))
* Report foreign struct/variant trees by field name, not opaque build errors ([c6d2274](https://github.com/git-ents/facet-git-tree/commit/c6d2274adc370b8649a74bc12a5ad5b1930106ba))
* Report the re-store remedy in MissingLeafNewline ([f5fdb13](https://github.com/git-ents/facet-git-tree/commit/f5fdb1393784d7da8ecdfc49717cbc50b3d6a810))
* Require struct trees and schema fields to correspond exactly ([a96b01d](https://github.com/git-ents/facet-git-tree/commit/a96b01d07c5f24dcd80f32fc4fca766cea2733e6))
* Serialize isize and usize scalar fields ([706d7eb](https://github.com/git-ents/facet-git-tree/commit/706d7eb2a4a0020130b6884bbc9ea4d92d073efc))
* Spec platform-width integers as i64/u64-shaped, not pointer-width-relative ([1a86cc1](https://github.com/git-ents/facet-git-tree/commit/1a86cc12ef749d96613e8afedce666a3eb5f378d))
* Strip the leaf newline in the code_review example dump ([f5fdb13](https://github.com/git-ents/facet-git-tree/commit/f5fdb1393784d7da8ecdfc49717cbc50b3d6a810))
* Use `into_list_like` instead of `into_ndarray` for `Def::Array` ([0a18d7a](https://github.com/git-ents/facet-git-tree/commit/0a18d7ab8333d3ccb215fbb22b89c84a8ccc441d))


### Performance Improvements

* Bulk-copy byte sequences instead of reflecting per byte ([9474e82](https://github.com/git-ents/facet-git-tree/commit/9474e82b8ec1e5116eb04a3ab7969ac814fb383b))
* Parse each sequence ordinal once via sort_by_cached_key ([b28f29d](https://github.com/git-ents/facet-git-tree/commit/b28f29dcf239092ffa9b25ca10ac7685350cb677))


### Documentation

* Add enums_and_options example ([2fcf013](https://github.com/git-ents/facet-git-tree/commit/2fcf013e54c1531d82b33d20f61e10e58a9d549f))
* Add prompt for evaluation ([29815bd](https://github.com/git-ents/facet-git-tree/commit/29815bdf18bd33ef2bb6264d14f79941b6b1169a))
* Add roundtrip example ([2fcf013](https://github.com/git-ents/facet-git-tree/commit/2fcf013e54c1531d82b33d20f61e10e58a9d549f))
* Add serialization examples ([2fcf013](https://github.com/git-ents/facet-git-tree/commit/2fcf013e54c1531d82b33d20f61e10e58a9d549f))
* Add struct_to_tree example ([2fcf013](https://github.com/git-ents/facet-git-tree/commit/2fcf013e54c1531d82b33d20f61e10e58a9d549f))
* Clarify dynamic value round trips ([7bcddd1](https://github.com/git-ents/facet-git-tree/commit/7bcddd1fd73e2c0afa3f2afade47fe9b5b89c928))
* Convert the documentation set from Markdown to AsciiDoc ([65e9e0f](https://github.com/git-ents/facet-git-tree/commit/65e9e0fe975cecc27b48bd60054bfcea188cfd83))
* Describe the codec entry in specification.adoc and update both recorded genesis ids ([684e173](https://github.com/git-ents/facet-git-tree/commit/684e17336802f5390bd54d701bc5e5da33cea588))
* Document byte-sequence and smart-pointer encoding in the specification ([849666a](https://github.com/git-ents/facet-git-tree/commit/849666a0c682884a78b945894216c57d24db84e0))
* Drop analysis and evaluation notes ([cc34ede](https://github.com/git-ents/facet-git-tree/commit/cc34ede0975fddbc86882d6dd67fb00a3e3bad98))
* Independent technical evaluation of the git-model database proposal ([c5215ad](https://github.com/git-ents/facet-git-tree/commit/c5215ad59692e9946ff4f9478343b46b37f81d81))
* Replace AI-prose doc comments with terse invariants ([354e021](https://github.com/git-ents/facet-git-tree/commit/354e021fc90933b2aea443349ce9b906eacb62bd))
* Replace type-zoo examples with domain-driven ones ([1d41124](https://github.com/git-ents/facet-git-tree/commit/1d41124d6b40d67cbdc8ee61f659fc3f2adddb2a))
* Respecify encoding as schemaless with externally-tagged enums ([873fca3](https://github.com/git-ents/facet-git-tree/commit/873fca3e00251b5444de47dfcc73699c514ee5c6))
* Specify scalar vs composite map key encodings ([fd59a0d](https://github.com/git-ents/facet-git-tree/commit/fd59a0d7bc90a82b028058ee577ce14c899ce57b))
* Specify SHA-1 object identity hash ([1aebbbc](https://github.com/git-ents/facet-git-tree/commit/1aebbbcf08b642d50bb52a05917fd79b1ecf6659))
* Specify variant, ordinal, leaf-byte, and boolean encoding ([1aebbbc](https://github.com/git-ents/facet-git-tree/commit/1aebbbcf08b642d50bb52a05917fd79b1ecf6659))
* State the encoding's subtle wins and their deliberate costs ([08a31e3](https://github.com/git-ents/facet-git-tree/commit/08a31e3355c44e18b6680bf65bf29d25d11da878))
* Tighten serializer comments ([f918cea](https://github.com/git-ents/facet-git-tree/commit/f918cea2f999b04572fa97fcdaf3d3978206b1b0))
* Use Arc&lt;[u8]&gt; for binary artifacts in the build_cache example ([849666a](https://github.com/git-ents/facet-git-tree/commit/849666a0c682884a78b945894216c57d24db84e0))


### Code Refactoring

* Batch of small cleanups from the adversarial review ([a396a51](https://github.com/git-ents/facet-git-tree/commit/a396a519b7b082c78976731ff7d2beec4376e6bf))
* Centralize shape classification ([db95028](https://github.com/git-ents/facet-git-tree/commit/db95028c6b0dd2a8aadc4ad7235aaa37ae7950f9))
* Centralize tree entry sorting ([2349b68](https://github.com/git-ents/facet-git-tree/commit/2349b6839cf2b50f03913271c133b941dfe40b92))
* Collapse repeated map_err into a `msg` helper ([d65b873](https://github.com/git-ents/facet-git-tree/commit/d65b8736bb6c0f96ad82ed2eaf745ba07dd2a8d1))
* Consolidate the codec's depth bounds into a limits module ([9704a74](https://github.com/git-ents/facet-git-tree/commit/9704a746e195ef377255e37346185762e7c8a927))
* Drop `Error::ReservedKey` and reserved-name checks in `check_key` ([873fca3](https://github.com/git-ents/facet-git-tree/commit/873fca3e00251b5444de47dfcc73699c514ee5c6))
* Encode Git trees as schemaless, type-driven values ([873fca3](https://github.com/git-ents/facet-git-tree/commit/873fca3e00251b5444de47dfcc73699c514ee5c6))
* Make Schema::from_shape_with_limit honestly public ([8ece960](https://github.com/git-ents/facet-git-tree/commit/8ece9601146a6e7c237ac9f379fcbbde8757b99f))
* Narrow the public surface ([38c710c](https://github.com/git-ents/facet-git-tree/commit/38c710c93e386d0f6f3b4568a90220cd0f3ddaea))
* One home for Path, is_scalar_schema, and value_kind ([490bc91](https://github.com/git-ents/facet-git-tree/commit/490bc91c50f9d8094424a2deae82a5a660a67f66))
* Remove `.schema` schema object and `.variant` sentinel ([873fca3](https://github.com/git-ents/facet-git-tree/commit/873fca3e00251b5444de47dfcc73699c514ee5c6))
* Rename Schema to Node and SchemaDoc to Schema ([350efa7](https://github.com/git-ents/facet-git-tree/commit/350efa70007e13f247ae39d7b7ba30bcce714e09))
* Share supported scalar mapping ([e62f239](https://github.com/git-ents/facet-git-tree/commit/e62f2398b070a3b2c06a576526461c180e6fa065))
* Simplify serialization entry points ([c7475fc](https://github.com/git-ents/facet-git-tree/commit/c7475fc0226b5d782ecd34a050d56f4c6146b13f))
* Sort_by_ordinal parses each entry name exactly once ([8b36d0b](https://github.com/git-ents/facet-git-tree/commit/8b36d0bda66bebe8ae9b1e9623c7dad52ab6778b))
* Split tree serializer branches ([d2fea0c](https://github.com/git-ents/facet-git-tree/commit/d2fea0c73192d921a3bdc2e35a5ce4bdaf8f68b8))


### Tests

* Add byte_sequences suite for Vec&lt;u8&gt;/[u8; N] blob encoding and dedup ([95fcea6](https://github.com/git-ents/facet-git-tree/commit/95fcea68a49c4af2d648e00941e23e26da57c9a9))
* Add initial integration tests ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))
* Add initial unit tests ([bc53766](https://github.com/git-ents/facet-git-tree/commit/bc53766b8c91fe89b7e7ee84d87ebbee5bb77c1c))
* Add pointers suite for Box/Rc/Arc transparency and Arc&lt;[T]&gt; slices ([95fcea6](https://github.com/git-ents/facet-git-tree/commit/95fcea68a49c4af2d648e00941e23e26da57c9a9))
* Add tests/codec_fixture.rs coverage and sharing checks, and rebuild the golden-oid tests around the codec-inclusive generation tree ([684e173](https://github.com/git-ents/facet-git-tree/commit/684e17336802f5390bd54d701bc5e5da33cea588))
* Cover byte-sequence and smart-pointer encoding ([95fcea6](https://github.com/git-ents/facet-git-tree/commit/95fcea68a49c4af2d648e00941e23e26da57c9a9))
* Cover depth, ordinal, and Option foreign-tree rejections ([d65b873](https://github.com/git-ents/facet-git-tree/commit/d65b8736bb6c0f96ad82ed2eaf745ba07dd2a8d1))
* Cover int-keyed and composite-keyed map serialization and round-trip ([fd59a0d](https://github.com/git-ents/facet-git-tree/commit/fd59a0d7bc90a82b028058ee577ce14c899ce57b))
* Cover scalar and leaf invariants ([95dbbf7](https://github.com/git-ents/facet-git-tree/commit/95dbbf7426452a90266f76db6981807bea2088b4))
* Enable collection and ordinal serialization tests ([0a18d7a](https://github.com/git-ents/facet-git-tree/commit/0a18d7ab8333d3ccb215fbb22b89c84a8ccc441d))
* Enable collection deserialization tests ([3432297](https://github.com/git-ents/facet-git-tree/commit/3432297139acc232bd0a28e380456bd31715a143))
* Enable variant and Option deserialization tests ([d4fff87](https://github.com/git-ents/facet-git-tree/commit/d4fff87bc0b06ff37fc32ce2276b64b77413945e))
* Enable variant serialization tests ([6517e44](https://github.com/git-ents/facet-git-tree/commit/6517e44bebbc193502814d09820564abdbb25689))
* Rename schema_cycles.rs to recursive.rs and align suites ([873fca3](https://github.com/git-ents/facet-git-tree/commit/873fca3e00251b5444de47dfcc73699c514ee5c6))
* Strengthen serialization properties ([c6c311c](https://github.com/git-ents/facet-git-tree/commit/c6c311cd95d839292140c40d3463dec9cc394ee5))
* Unskip all variants.rs tests ([6517e44](https://github.com/git-ents/facet-git-tree/commit/6517e44bebbc193502814d09820564abdbb25689))
* Unskip option_some_roundtrip and option_none_roundtrip in roundtrip.rs ([d4fff87](https://github.com/git-ents/facet-git-tree/commit/d4fff87bc0b06ff37fc32ce2276b64b77413945e))
* Unskip recursive_type_serializes and recursive_type_roundtrips ([3432297](https://github.com/git-ents/facet-git-tree/commit/3432297139acc232bd0a28e380456bd31715a143))
* Unskip vec_equality and vec_order_matters in structural.rs ([3432297](https://github.com/git-ents/facet-git-tree/commit/3432297139acc232bd0a28e380456bd31715a143))
* Unskip vec/array/map roundtrip tests in roundtrip.rs ([3432297](https://github.com/git-ents/facet-git-tree/commit/3432297139acc232bd0a28e380456bd31715a143))
