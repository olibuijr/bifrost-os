# Wave 5: Core Search (CoreSpotlight) index delegate and named index descriptions

Two gap rows of the Core Spotlight framework, built in pure Rust with no new crates, unsafe code or services.
Both were `gap` in `docs/parity-matrix/data/core-spotlight.json`; they are now `partial`.

## What was built

- **Index descriptions** (`springboard/ui/src/searchindex.rs`, re-exported from `bifrost_app::quicksearch`):
  an index name check (ASCII letters, digits, `-`, `_` and an inner `.`, 1 to 64 bytes, no leading `.`), the four
  protection classes with their raw names, an `IndexDescription` (name plus optional protection class), and the store
  file name of a named index (`<app>@<name>.json`, the default index stays `<app>.json`).
- **Index client delegate** (`springboard/sdk/src/quicksearch.rs`): the `CSSearchableIndexDelegate` trait with
  `reindex_all_searchable_items` and `searchable_items_for`, both defaulting to no items, and the app-run entry points
  `CSSearchableIndex::reindex_all_with` (clears the index, then indexes the delegate's items) and
  `CSSearchableIndex::reindex_identifiers_with` (indexes the delegate's items for the given identifiers, keeps the rest).
  Refused items are reported as in `index_checked`.

## Official API mirrored

- `CSSearchableIndexDelegate`: `searchableIndex(_:reindexAllSearchableItemsWithAcknowledgementHandler:)`,
  `searchableIndex(_:reindexSearchableItemsWithIdentifiers:acknowledgementHandler:)`,
  `searchableItems(forIdentifiers:searchableItemsHandler:)`.
  Doc: https://developer.apple.com/documentation/corespotlight/cssearchableindexdelegate
- `CSSearchableIndexDescription`: `protectionClass`.
  Doc: https://developer.apple.com/documentation/corespotlight/cssearchableindexdescription
- `CSSearchableIndex(name:protectionClass:)`, referenced for the naming rule.
  Doc: https://developer.apple.com/documentation/corespotlight/cssearchableindex

## REA evidence

- `ev_efe53d7aa061ce6feeeaa7ce94710bf52cdbaafc67c077e91d828f22085eda41`: CSSearchableIndexDelegate (rendered page).
- `ev_c28b2a2411af1581e7e4b9805e8e6d19be514ef7b095940659e3a34d80f86f7e`: CSSearchableIndexDescription (rendered page).

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| core-spotlight | Index client delegate | partial | springboard/sdk/src/quicksearch.rs (`CSSearchableIndexDelegate`, `reindex_all_with`, `reindex_identifiers_with`) |
| core-spotlight | Index descriptions | partial | springboard/ui/src/searchindex.rs (`IndexDescription`, `ProtectionClass`, `validate_name`, `store_file_name`) |

## Gaps that remain

- The shell never asks an app to reindex, so the delegate runs only when the app calls it. Index extensions
  (`CSIndexExtensionRequestHandler`) stay a gap.
- The shell keeps only the default index per app. A named index is recorded and its store file name is computed, but
  items are still written to the default store, and the protection class is not applied to the file on disk.
- No index `indexDelegate` property, no throttling callbacks (`searchableIndexDidThrottle`), and no data or file-URL
  callbacks for drag and drop.
- The Foundation models search tool row stays a gap (no app-facing way to pass indexed content to the assistant).

## Tests

- `springboard/ui/src/searchindex.rs`: 5 tests (name rules and boundaries, raw protection names, description builder,
  store file names).
- `springboard/sdk/src/quicksearch.rs`: 5 new tests (reindex all clears then indexes, identifier reindex keeps other
  items, unknown identifiers send nothing, the default delegate only clears, refused items are reported by position).
- Not built or run in this wave (no builds were allowed); expected values were worked out from the code.
