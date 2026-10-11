# SwiftData parity

Framework: SwiftData, official documentation https://developer.apple.com/documentation/swiftdata (macOS 14.0 and later).
Status after this round: `partial` (was `gap`). Matrix row: `docs/parity-matrix/data/swiftdata.json`.

## What was built

Three pure models in `springboard/ui/src` (bifrost-ui, std only, no unsafe, no new crates). Each has its own
`#[cfg(test)] mod tests`.

- `relations.rs`: relationships with delete rules and count limits. `DeleteRule` has the four official cases
  (`cascade`, `deny`, `nullify`, `noAction`). `Relation` carries an owner entity, a name, a rule, a minimum and an
  optional maximum count. `Relations` keeps the links and provides `link`, `unlink`, `targets`, `referrers` (the
  inverse lookup), `check_counts`, `plan_delete` and `apply_delete`. Tests: 7.
- `history.rs`: transaction history with tokens. `Token` only grows. `Change` is `Insert`, `Update` or `Delete` for a
  record. `History::commit` records one transaction and returns its token, and an empty change list records nothing.
  `Descriptor` (after, author, limit) drives `fetch`. `prune` drops old transactions and keeps later tokens. Tests: 5.
- `model_store.rs`: a model-context style record store. `Store` offers `insert`, `set`, `get`, `fetch`, `link`,
  `unlink`, `delete`, `save`, `rollback` and `has_changes`. Unique fields are checked on every write. `save` checks the
  relation minimums, diffs the working records against the last save, and commits one transaction. Tests: 6.

Module lines were added to `springboard/ui/src/lib.rs` (`pub mod relations;`, `pub mod history;`, `pub mod model_store;`).

## Official API names mirrored

| Official name | Bifrost |
|---|---|
| `Schema.Relationship.DeleteRule` (`cascade`, `deny`, `noAction`, `nullify`) | `relations::DeleteRule` |
| `Relationship(_:deleteRule:minimumModelCount:maximumModelCount:originalName:inverse:hashModifier:)` | `relations::Relation` (owner, name, rule, `min`, `max`); no inverse argument |
| `ModelContext` (`insert`, `delete`, `save`, `rollback`, `hasChanges`, `fetch`) | `model_store::Store` |
| `Schema.Unique` (single field) | `Store::declare_unique` (single field only) |
| `HistoryDescriptor` (`predicate`, `sortBy`, `fetchLimit`) | `history::Descriptor` (`after`, `author`, `limit`); no sort key |
| `HistoryTransaction` (`token`, `author`, `timestamp`, `changes`) | `history::Transaction` (`token`, `author`, `at`, `changes`) |
| `HistoryChange.insert`, `.update`, `.delete` | `history::Change::{Insert, Update, Delete}` |
| `DefaultStore.TokenType` (history token) | `history::Token` |

The delete rules follow the official descriptions. A rule applies to a relation's targets when its owner is deleted.
`cascade` deletes the targets, `deny` refuses the delete while targets exist, `nullify` clears the targets' references
back to the deleted owner, and `noAction` leaves the targets alone. A reference that no rule clears stays in place,
and callers skip ids that no longer exist.

## REA evidence

- `ev_7fb2e7d90731630b79a0b66327b6704203a145229cdc3fda968d9cc44c8b5c6e`: REA passive inspection of the rendered
  SwiftData landing page (title, topics, macro list, `ModelContext`, `Schema.Relationship`).
- `ev_563b0360c56d3e1d3ca6e344e289dcc2215e1be7433634f4f0175d1860ff401e`: the evidence id already on the matrix row.

Official DocC used for API shape: `swiftdata/modelcontext`, `swiftdata/schema/relationship`,
`swiftdata/schema/relationship/deleterule-swift.enum`, `swiftdata/historydescriptor`, `swiftdata/historychange`,
`swiftdata/historytransaction`, `swiftdata/modelcontainer`, `swiftdata/defaultstore`.

## Remaining gaps

- No declarative model type or schema macro. Entities and fields are strings.
- `bifrost_ui::model_store` is not yet exposed through `bifrost_app`, so apps cannot use it.
- The store is in memory only: no on-disk format, no store abstraction, no batching.
- Fetch takes a Rust closure. There is no predicate, no sort descriptor, no sections, and no live results.
- Relation inverses are not declared. Only `referrers` finds them. Uniqueness covers one field, with no index structures.
- No history observer, no persistence or merge of the transaction log, and no sync transport.
- Undo does not record store changes automatically. Rollback returns to the last save only.
- No model actor and no isolation of model access.

## Verification

Not built here by design: the owner's build stage runs the tests. Names were checked with `python3 tools/check-names.py`,
which reported no hits.
