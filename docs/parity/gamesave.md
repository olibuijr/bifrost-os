# GameSave parity

Branch `parity/gamesave`. Framework: GameSave (https://developer.apple.com/documentation/gamesave), macOS 26.0+.

## What was built

Two pure-Rust modules, std only, no network and no sync engine:

- `springboard/ui/src/save_sync.rs` (`bifrost_ui::save_sync`): the state and conflict model. `State` has the seven
  cases of the official directory state (closed, conflicted, error, local, offline, ready, syncing). `State::is_usable`
  says which states let an app read and write. `Version` carries the saving device, the modification time, whether it
  is local, and a content fingerprint. `resolve_conflicts(state, with)` keeps a chosen version and returns `Ready`, or
  refuses with `ConflictError`. `newest` picks the version to suggest first.
- `springboard/sdk/src/save_sync.rs` (`bifrost_app::save_sync`): `SyncedDirectory`. `open(container_id, data_root)`
  creates `<data_root>/saves/<container_id>` after checking the identifier. `url`, `state`, `finish_syncing`,
  `trigger_pending_upload`, `list_files`, `read_file`, `write_file` (temp file, fsync, rename, fsync of the folder),
  `remove_file` and `close`. `SaveError` with `domain()` returning `GameSaveErrorDomain`.

Both modules are registered with one `pub mod save_sync;` line in their crate's `lib.rs`.

## Official API names mirrored

- `GameSaveSyncedDirectory`, `openDirectory(containerIdentifier:)`, `state`, `close()`
- `GameSaveSyncedDirectory.State`: `closed`, `conflicted(versions:)`, `error(_:)`, `local(_:)`, `offline(_:)`,
  `ready(_:)`, `syncing`
- `GameSaveSyncedDirectory.Version`: `url` (as `SyncedDirectory::url`), `isLocal`, `localizedNameOfSavingComputer`,
  `modifiedDate`
- `resolveConflicts(with:)`, `finishSyncing()`, `triggerPendingUpload()`
- `GameSaveErrorDomain`

Payloads that the documentation leaves open are our choice: `error` carries a message, `local`, `offline` and `ready`
carry a `Version`, and `conflicted` carries its versions. The device name is read from `/proc/sys/kernel/hostname`.

## Official documentation evidence

- Landing page: REA `ev_38b189d77af9ec8c6f270dc1252ea7010c5ceaacc127d8288101748d90ed4c99` (rendered
  `https://developer.apple.com/documentation/gamesave`). The earlier matrix row cited
  `ev_fcb07673c7548ce7ddb7c1b1a701fccb1390da8285cedfcac134310f6a43a96e`.
- `docc.py` reads: `gamesave/gamesavesynceddirectory` (members and their order), `.../state-swift.enum` (the seven
  cases with their descriptions), `.../version` (`url`, `isLocal`, `localizedNameOfSavingComputer`, `modifiedDate`),
  `.../opendirectory(containeridentifier:)`, `.../resolveconflicts(with:)`.

## Status changes in the matrix

`docs/parity-matrix/data/gamesave.json`: the framework is now `partial`. Rows set to `partial`: synced directory,
syncing and progress, conflict resolution, triggering uploads, offline saving and error domain. Nothing is `parity`,
because no sync engine exists. The Objective-C row stays `n/a`, and the cloud backup row stays `partial`.

## Remaining gaps

1. A cloud sync engine behind the directory. Without it, `Ready`, `Offline`, `Syncing` and `Conflicted` never occur.
2. Version history on disk and cloud versions, so that a real conflict can arise and be resolved by an app.
3. The sync progress alert and a pending-upload queue behind `trigger_pending_upload`.
4. Mapping per-app save directories into bifrost-cloud backups. No SDK or client path exists.

## Tests

`springboard/ui/src/save_sync.rs` has six tests (usable states, version read-back, conflict resolution both ways,
unknown version and no-conflict refusals, `newest` ordering, error messages). `springboard/sdk/src/save_sync.rs` has
six tests (container validation, write and read round trip, hidden temp files and refused names, fingerprint changes
with contents, closed directory refusals, no-op sync and error domain). Per the workflow, nothing was built or run
here; the next stage builds and tests.
