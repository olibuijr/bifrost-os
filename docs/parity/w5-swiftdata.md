# Wave 5: SwiftData parity (model schema and data store)

Source: https://developer.apple.com/documentation/swiftdata (matrix row set in `docs/parity-matrix/data/swiftdata.json`).

## What was built

Two gap rows were picked because they can be built as pure Rust models with no disk, network or hardware access.

- `springboard/ui/src/model_schema.rs` (new): per-field options `Attribute { unique, indexed, transient }`, a `Schema`
  that declares them per entity, lists unique and indexed fields, validates that a write uses only declared fields
  (for entities that have a schema), and drops transient fields from what a backend persists. Mirrors the
  `Attribute(_:originalName:hashModifier:)`, `Unique(_:)`, `Index(_:)` and `Transient()` options of the official
  model definition. No macro: apps declare options at runtime.
- `springboard/ui/src/data_store.rs` (new): a `DataStore` trait (fetch by entity, fetch count, fetch identifiers,
  save a batch, erase), a `Batch` of ordered `Write::Put` and `Write::Remove` writes, and `MemoryDataStore`. A batch
  applies to a copy and is kept only if every write succeeds, so a refused batch changes nothing. Mirrors
  `DataStore`, `DataStoreBatching` and the fetch and save-changes requests of the official storage protocol.
  File-backed storage is not built here: it belongs outside the UI crate.
- `springboard/ui/src/model_store.rs` (changed): the store now owns a `Schema`. `declare_attribute` declares a field
  and, for a unique field, registers the uniqueness check. `insert` and `set` refuse fields the schema does not
  declare (new `StoreError::Schema`). `persistent_fields(id)` returns the non-transient fields of a record.
- `springboard/ui/src/lib.rs`: one `pub mod` line each for `model_schema` and `data_store`.

Tests: `model_schema` has 5 behaviour tests, `data_store` has 6, and `model_store` gained 2 (unknown-field refusal
and transient exclusion) on top of its 6 existing tests. Expected values were worked out by hand from the rules.
No builds were run in this wave (per the wave rules).

## Official API names mirrored

- `Attribute(_:originalName:hashModifier:)`, `Unique(_:)`, `Index(_:)`, `Transient()` (model definition)
- `DataStore`, `DataStoreBatching`, `DefaultStore` (model storage, the trait and batch parts only)

## REA evidence

- `ev_cbc31ff9109bb4d6931fbdefdf7bc05280afd37ffbef458cf6eb6759cd72d6e2` (SwiftData landing page)
- `ev_33ee3ddf007c42935b49c09902f07addf2e05f412f573cea62c7eda9af316e82` (DataStore page)

## Matrix rows closed

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| SwiftData | Model classes and schema (@Model, Attribute, Unique, Index, Transient) | partial | springboard/ui/src/model_schema.rs |
| SwiftData | Data store abstraction (DataStore, DefaultStore, custom stores, batching) | partial | springboard/ui/src/data_store.rs |

Still missing for both rows: a declarative model macro, index-backed lookups, a file-backed default store and a
SDK-level API; see the matrix `gaps` for SwiftData.
