# Background Assets parity

Official reference: https://developer.apple.com/documentation/backgroundassets (macOS 13.0 and later).
REA evidence of the rendered page: `ev_8f5cabfa05491f21ef46bed9836bfa85cc4c0c72908f41a363d622ca56011b78`.
The API shapes were read from the official documentation with `tools/parity/docc.py` (not REA evidence) for
`backgroundassets`, `backgroundassets/sizecalculationmethod`, `backgroundassets/assetpackmanager` and
`backgroundassets/badownloadmanager`.

## What was built

Two pure Rust models in `springboard/ui/src` (`bifrost-ui`, std only, no I/O):

- `assetpack.rs` mirrors the managed asset-pack API. It holds `AssetPack` (id, version, download size, file lengths,
  languages), `AssetPackManifest` (unique, non-empty ids), `AssetPackManager` (local state per pack, `status`,
  `mark_downloaded`, `remove`, `local_size`, `needed`, `resolved_language`, `packs_for_languages`), and
  `SizeCalculationMethod` with the two official cases, `Logical` and `Allocated`.
- `downloadqueue.rs` mirrors the download manager queue. It holds `DownloadQueue` (`schedule`, `next`, `current`,
  `record_progress`, `remaining_bytes`, `complete`, `cancel`), `Priority` (essential first), `Budget` (an essential
  maximum and a total maximum, matching the concept of the initial download restrictions), and a line format
  (`to_lines` / `from_lines`) so a queue can be saved and restored.

Both are declared in `springboard/ui/src/lib.rs`. Tests sit in each module (7 in `assetpack.rs`, 8 in `downloadqueue.rs`).

## Mapping to the official names

| Official name | BifrOSt model |
|---|---|
| `AssetPack`, `AssetPackManifest`, `AssetPackManager` | `AssetPack`, `AssetPackManifest`, `AssetPackManager` |
| `AssetPack.Status` (local status) | `PackStatus` (NotDownloaded, Available, UpdateAvailable); the official case names were not read, so they are not mirrored |
| `remove(assetPackWithID:)` | `AssetPackManager::remove` (returns freed logical bytes) |
| `localSize(ofAssetPackWithID:calculationMethod:)` | `AssetPackManager::local_size` |
| `SizeCalculationMethod.logical` / `.allocated` | `SizeCalculationMethod::Logical` / `Allocated` |
| `resolvedLanguage`, `reconcilePreferredLanguages()` | `AssetPackManager::resolved_language` and `packs_for_languages` (selection only; no reconciliation with the live locale) |
| `BADownloadManager.scheduleDownload`, `cancel`, `fetchCurrentDownloads` | `DownloadQueue::schedule`, `cancel`, `current` |
| Essential and non-essential downloads | `Priority::Essential`, `Priority::NonEssential` |
| `BAInitialDownloadRestrictions`, `BAEssentialMaxInstallSize`, `BAMaxInstallSize` | `Budget { essential_max_bytes, max_bytes }`, checked by `schedule` |

## Behaviour notes

- Budgets count queued downloads only. A download that completes or is cancelled frees its share. The official
  budgets apply to the install size before first launch, which BifrOSt does not do yet.
- `Allocated` size rounds each file up to the block size. A block size of 0 uses the logical size.
- `packs_for_languages` keeps every unlocalized pack and only the localized packs for the one resolved language.
- The queue's saved format is tab-separated: id, url, bytes, received, `E` or `N`. Ids and URLs must not contain tabs or
  newlines, and a malformed line is an error with its line number.

## Remaining gaps

- No SDK binding: `bifrost_app` does not yet expose the pack manager or the queue, and no app can list or read packs.
- No transfer: the queue is not driven by `download_task` in `springboard/sdk/src/url.rs`, and no app stores its queue.
- No scheduling while the app is closed, no downloader extension and no manifest URL.
- No install-size enforcement on disk, and no restrictions before first launch.
- No mock server for testing packs locally.
- Apple-hosted packs and App Store Connect upload stay `n/a`: the PC App Store has no hosted asset service.
