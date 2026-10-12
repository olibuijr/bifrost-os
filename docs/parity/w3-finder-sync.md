# Wave 3: Finder Sync gap rows (file sync state for the Files app)

Framework: Finder Sync (https://developer.apple.com/documentation/findersync), macOS 10.10 and later.

## What was built

Two pure Rust models in `springboard/ui/src` (std only, no unsafe, no I/O). Neither is wired into the Files app
or the SDK yet, so the matrix rows below move to `partial`, not `parity`.

- `springboard/ui/src/file_sync.rs` (`file_sync::SyncController`): the state a sync provider keeps for the file
  manager. It holds the managed directories, the directories the user is looking at (begin and end observing,
  with subdirectory matching), badge images and labels per identifier, a badge identifier per item under a
  managed directory, the selected items and the target folder. Managed matching compares path components, so
  `/a/bc` is not under `/a/b`. Replacing the managed directories drops observations and item badges outside the
  new set.
- `springboard/ui/src/item_attrs.rs` (`item_attrs::ItemStore`): per-path opaque tag data and last-used time
  (Unix seconds, no clock). It returns the most recent items (time descending, then path ascending) and forgets
  all attributes of an item. An item with no attribute holds no entry.

Both are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`. Each has a `#[cfg(test)] mod tests`
with six behaviour tests whose expected values follow from the rules above.

Not built: a menu contribution model (`FIMenuKind`), toolbar items, the extension controller and enablement, and
the extension host. These stay `gap`.

## Official API names mirrored

- `FIFinderSyncController`: `directoryURLs` (`SyncController::directories`, `set_directories`), `selectedItemURLs()`
  (`selected_items`, `set_selection`), `targetedURL()` (`targeted`, `set_selection`), `setBadgeIdentifier(_:for:)`
  (`set_badge_identifier`), `setBadgeImage(_:label:forBadgeIdentifier:)` (`set_badge_image`), `tagDataForItem(with:)`
  and `setTagData(_:forItemWith:completion:)` (`ItemStore::tag_data`, `set_tag_data`), `lastUsedDateForItem(with:)` and
  `setLastUsedDate(_:forItemWith:completion:)` (`ItemStore::last_used`, `set_last_used`).
- `FIFinderSyncProtocol`: `beginObservingDirectory(at:)` and `endObservingDirectory(at:)` (`begin_observing`,
  `end_observing`, `is_observing`). `requestBadgeIdentifier(for:)` is covered by `badge_identifier` and `badge_for`
  read side only; the provider callback itself is not modelled.

Differences from Apple: the badge image is a name (`String`), not an image object; dates are `u64` Unix seconds,
not `Date`; completion handlers are absent because the calls are synchronous; tag data is an opaque byte vector
(`Data?`).

## REA evidence

- `ev_f3f24717e6ac18f6d1dba32e8abd434c92851592ad666ad96d263c56c0764b88`: Finder Sync landing page (recorded in wave 1).
- `ev_40718c08118a3803b64e6ad8c299c5952779602e24478a2a6d552489dc8b9990`: FIFinderSyncController page, rendered
  headings and method list checked against the models above.
- `ev_9f6bee5bea49507aa9a0445391518f09bc9d7479a89afabdb999df7251a05501`: FIFinderSyncProtocol page. The capture
  returned no title or rendered text, so the method list was taken from `tools/parity/docc.py findersync/fifindersyncprotocol`
  instead. Treat this id as weak evidence.

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| Finder Sync | Badges on files and folders | partial | springboard/ui/src/file_sync.rs |
| Finder Sync | Observing monitored directories | partial | springboard/ui/src/file_sync.rs |
| Finder Sync | Current selection and target folder | partial | springboard/ui/src/file_sync.rs |
| Finder Sync | Per-item tags and last-used data shared with the Finder | partial | springboard/ui/src/item_attrs.rs |

The row feature text is copied from `docs/parity-matrix/data/finder-sync.json`. The status is partial because
neither model is connected to the Files app or to an SDK entry point, and no test has run.

## Not verified

No cargo build or test was run, per the wave rules. The code was checked by reading only.
