# Core Data parity

Official reference: https://developer.apple.com/documentation/coredata (macOS 10.4 and later).
Matrix row data: `docs/parity-matrix/data/core-data.json`.

REA evidence of the rendered landing page: `ev_303310d8bdab28a8d9fdea3553290d527e2f848744db028bc4cd0293734f46ce`.
The API shape comes from the official DocC pages for `coredata/nsfetchrequest`, `coredata/persistent-history` and
`coredata/migrating-your-data-model-automatically`, read with `tools/parity/docc.py`.

## What was built

Three pure Rust models, std only, with unit tests. Each mirrors one missing feature of the framework. None of them
replaces an existing store.

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/fetch.rs` | `NativeFetchRequest`, `NativePredicate`, `NativeSortDescriptor`, `fetchLimit`, `fetchOffset` | A `Record` trait exposes named fields. `Predicate` holds comparisons (`Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge`), case-insensitive text containment, and `and`, `or`, `not`. `FetchRequest` adds stable multi-key sorting, an offset and a limit. `fetch` and `fetch_indices` run it over a slice. |
| `springboard/ui/src/history.rs` | `NativePersistentHistoryTransaction`, `NativePersistentHistoryChange`, `NativePersistentHistoryToken` | `History` appends one `Transaction` per save with its `Change` list (insert, update, delete) and returns a monotonic `Token`. Readers ask for transactions `since` a bookmark. `purge_through` drops seen transactions, and `is_complete_since` tells a reader whose bookmark predates the purge that it missed changes. |
| `springboard/core/src/migrate.rs` | Lightweight and staged migration chain | `Plan<T>` holds one step per version, each step produces `from + 1`. `migrate` runs the steps from the stored version to the current one. Data from a newer build is refused with `NewerThanBuild`. `check` finds gaps in the chain. Step failures name the step and its version. |

Naming follows the neighbouring code: `Native` prefixes in comments, no platform names in `.rs` files
(`python3 tools/check-names.py` is clean).

## What it does not do

- No store uses any of these yet. The contacts and calendar stores still filter their own vectors, still write whole
  files, and still reject foreign headers instead of running the migration chain.
- No save writes to `History`, so no transactions are produced at run time.
- Fetch requests have no batch size, no relationship key paths and no entity names.
- No object graph, relationships, managed object context, dirty tracking or faulting.
- No fetched results controller that binds a query to a list.
- No history merging between devices, no CloudKit-style mirroring and no record sharing. These are tied to Apple's
  accounts and are `n/a` on this platform. Sync exists only as server-side counters in `bifrost-cloud`.

## Verification

- The unit tests cover equality, missing fields, text containment, combinators, multi-key sorting with direction, stable
  ties, offset and limit, mixed-kind ordering, token sequencing, purge and incomplete-reader detection, migration order,
  start-from-version, newer-data refusal, failing steps, and gap detection.
- Per the stage rules, nothing was compiled or tested in this round. The next stage must run
  `cargo test -p bifrost-ui fetch history` and `cargo test -p bifrost-core migrate`.

## Remaining gaps

1. Wire `History` into a store's save path, so each save records its changes.
2. Run the contacts and calendar stores through `migrate::Plan`, replacing the header check for older files.
3. Bind a `FetchRequest` to a table through `diff.rs`, which would close the fetched results controller row.
4. Add an entity-level model with relationships, which the Codable tree does not provide.
