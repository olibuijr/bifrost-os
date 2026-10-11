# CloudKit parity

Branch `parity/cloudkit`. Framework: CloudKit (https://developer.apple.com/documentation/cloudkit), macOS 10.10+.

## What was built

Three pure-Rust models in `bifrost-ui`, std only, no network, no disk and no service. They define the record
database, the sync queue and the asset store that a remote service would also implement. Nothing here talks to
iCloud or to bifrost-cloud, so every behaviour is local and in memory.

- `springboard/ui/src/record_store.rs` (`bifrost_ui::record_store`): `Database` with a `Scope` label and named zones
  (the default zone always exists). `Record` is a typed field dictionary (`Value`: string, int, real, bool, date,
  reference, asset) with a change tag and changed keys. `Reference` links records in one zone, with `delete_self`
  cascading deletes. `Predicate` (all, equals, not, and, or) drives `query` by record type. `save_expecting` takes a
  change-tag precondition (`Some(0)` creates, `Some(t)` replaces only if still `t`). `changes_since` returns a delta
  with the latest state of each record and the deleted IDs, and `snapshot` returns everything. The log is bounded;
  a token older than it fails with `ChangeTokenExpired`.
- `springboard/ui/src/record_sync.rs` (`bifrost_ui::record_sync`): `ErrorCode`, the full `CKError.Code` set from the
  official page, with `is_transient`; `RecordError` with `retry_after`. `SyncEngine` keeps a per-zone queue of saves
  and deletes, coalescing repeated edits (the first base tag is kept), pushes them (`push`), turns a stale save into a
  `Conflict` that holds the local and server versions, settles it with `ConflictPolicy` (`KeepLocal`, `KeepServer`,
  `Merge`), and pulls changes by token (`pull`, which falls back to a snapshot). `sync` does push then pull.
- `springboard/ui/src/record_asset.rs` (`bifrost_ui::record_asset`): `AssetStore` keeps file bytes once per content
  digest (FNV-1a 64, not a security hash; a digest clash with different bytes is an error, not a merge). `Asset` is the
  handle a record holds (digest, file name, size). `put` checks the name and a local size limit (`MAX_ASSET_BYTES`,
  250 MiB, our choice); `retain` drops bytes no asset names.

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`. No SDK wrapper was added: apps
reach these types through `bifrost_ui`, and a `springboard/sdk/src` re-export is left for a later round.

## Official API names mirrored

- `CKDatabase` (scope, zones, fetch, save, delete, query, changes), `CKDatabase.Scope` (public, private, shared as
  `Scope`)
- `CKRecord`, `CKRecord.ID` (`RecordID`), `init(recordType:recordID:)`, `setObject(_:forKey:)`, `changedKeys()`,
  `recordChangeTag`
- `CKRecord.Reference` (`Reference`, `deleteSelf` action as `delete_self`)
- `CKRecordZone`, `CKRecordZone.ID` (zone name in `RecordID`), `default()` (`DEFAULT_ZONE`)
- `CKQuery` (`recordType`, `predicate`); sort descriptors are not mirrored
- `CKAsset` (`fileURL` as `Asset::file_name` plus bytes in the store)
- `CKError`, `CKError.Code` (all codes listed on the official page, `ErrorCode`), `retryAfterSeconds`
- `CKModifyRecordsOperation.RecordSavePolicy` (`ifServerRecordUnchanged` as `Some(t)` and create-only as `Some(0)`
  in `save_expecting`; `changedKeys` merge as `ConflictPolicy::Merge`)
- Sync by change token: the store's `changes_since` and `ChangeSet` stand in for the zone change and fetch calls;
  they are not `CKSyncEngine` itself

## REA evidence

Official rendered pages, captured with `tools/parity/rea-page.sh`:

- CloudKit landing: `ev_78a8eeaf9597a15e1cf2f1798943bb89c16ee7d555be6be4f99fe94857f32244`
- CKRecord: `ev_2406b105262112cbedf8ed22191ac35732bc79f9ddd159fce65f5ca4df0d0896`
- CKRecordZone: `ev_602f9736a7b0ba47ae8ccbe782d8b737be12453a794ea406890ed314e0bda46e`
- CKQuery: `ev_4a4026db43ba052b46887671f1ac68de6a6b0ac40cb91d39d52f8edbe8b3b1d5`
- CKError: `ev_35c910b7067264e70e80b4b7298fa8e322947ca058d15e9c688eba6edc2352b3`
- CKAsset: `ev_ae6c5a283efa42f7973a7680a4df4cadeb807b39046353007be9b803f78d6480`
- CKDatabase: `ev_d3d1e5177f33b390a4640dbe66de3fb8c0f6007935163e9db45feedb247d594b`

`CKSyncEngine` has no DocC page at the path the tool tried (HTTP 404), so no REA page was captured for it and it is
not mirrored by name.

## Matrix rows closed

Status is `partial` for every row: each has the core model, and each lacks the server, subscriptions or auth.

| Framework | Feature (as in `docs/parity-matrix/data/cloudkit.json`) | New status | BifrOSt path |
|---|---|---|---|
| cloudkit | Containers and databases (public, private, shared) | partial | springboard/ui/src/record_store.rs (`Database`, `Scope`, zones) |
| cloudkit | Schemas, record zones and records | partial | springboard/ui/src/record_store.rs (`Record`, `Value`, `Reference`, `Predicate`) |
| cloudkit | Assets | partial | springboard/ui/src/record_asset.rs (`AssetStore`, `Asset`) |
| cloudkit | Sync engine, change tokens, subscriptions | partial | springboard/ui/src/record_sync.rs (`SyncEngine`); change tokens in springboard/ui/src/record_store.rs (`changes_since`) |
| cloudkit | Conflict handling | partial | springboard/ui/src/record_sync.rs (`Conflict`, `ConflictPolicy`, `resolve`) |
| cloudkit | Errors | partial | springboard/ui/src/record_sync.rs (`ErrorCode`, `RecordError`) |

Still open: shared records (`CKShare`), subscriptions and push, user identity, web auth tokens, and encrypted values.
The local backup and keychain rows stay as they were.

## Gaps

- No server. The store is local and does not sync with bifrost-cloud.
- Subscriptions and push are not built.
- Change tags and tokens are our own sequence numbers, not opaque server tokens.
- Batch partial failures (`partialErrorsByItemID`) are not reported per item: a batch is applied one record at a time.
- Conflicts carry no ancestor record; the merge uses the server version plus the local changed keys.
- Retries have no timer; the caller calls `push` again. `retry_after` is only carried on the error.
- No sort descriptors, result cursors or field indexes on queries.
- Share, identity and web-token APIs have no counterpart.

## Verification

Per the parity rules, nothing was built or run. The owner-rule check `python3 tools/check-names.py` passed after one
doc-comment fix. Each module has behaviour tests in a `#[cfg(test)] mod tests` block (7 in `record_store`, 9 in
`record_sync`, 6 in `record_asset`). They have not been run.
