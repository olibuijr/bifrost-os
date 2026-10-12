# Wave 5: PhotoKit parity (smart albums, thumbnail caching)

Branch `parity/w5-photokit`, based on `parity/integration` (waves 1 to 4). Two `os` rows of the PhotoKit matrix
(`docs/parity-matrix/data/photokit.json`) get pure Rust models in `springboard/ui/src/`. No builds or tests were run
in this wave; the code is checked by reading only.

## What was built

### `springboard/ui/src/photo_smart_albums.rs`

The smart albums of a photo library, as fixed rules over each asset's state:

- `SmartAlbum` with `Screenshots`, `RecentlyAdded`, `Favourites`, `RecentlyDeleted`, and `SmartAlbum::ALL` and `title()`.
- `Asset` (id, name, favourite, trashed, created, deleted_at) and `contains(album, asset, now)`.
- `members(album, assets, now)` returns the matching assets in input order.
- Rules: screenshots are name prefix `screenshot` (any case) and not trashed. Recently added is created within
  `RECENT_DAYS` (30) before `now` and at most one day in the future, not trashed. Favourites are marked and not
  trashed. Recently deleted is trashed. Trashed assets belong to no album except Recently Deleted.
- `purge_due(deleted_at, now)` and `days_left(deleted_at, now)` with `TRASH_DAYS` (30). A deletion time after `now`
  counts as just deleted.

Mirrors the smart album subtypes `smartAlbumScreenshots`, `smartAlbumRecentlyAdded` and `smartAlbumFavorites`.
The recently deleted album has no subtype in the official list and is a BifrOSt album.

### `springboard/ui/src/photo_caching.rs`

A model of the caching image manager: which thumbnail preparations are started.

- `Key` (asset, width, height, content mode, delivery mode). The same asset at two sizes is two preparations.
- `DeliveryMode` (`Opportunistic` default, `HighQualityFormat`, `FastFormat`) and `ContentMode` (`AspectFit`, `AspectFill`).
- `Manager`: `start(&keys)` returns the number newly started, `stop(&keys)` the number cancelled, `stop_all()`,
  `is_started`, `len`, `is_empty`. `allows_caching_high_quality_images` and its setter: when false, `start` skips
  keys that ask for `HighQualityFormat`. This reading of the property is a BifrOSt rule; the official page gives only
  the summary "determines whether the image manager prepares high-quality images".
- `preheat_window(visible, count, ahead)` widens the visible rows by `ahead` rows on each side, clamped to the list.
- `changes(old, new)` returns the indexes to start and to stop when the window moves.

Mirrors `startCachingImages(for:targetSize:contentMode:options:)`, `stopCachingImages(for:targetSize:contentMode:options:)`,
`stopCachingImagesForAllAssets()`, `allowsCachingHighQualityImages`, and the delivery modes
`opportunistic`, `highQualityFormat`, `fastFormat`.

Both modules are registered in `springboard/ui/src/lib.rs` with one `pub mod` line each.

## Not done

- No SDK wiring. Apps still get no API for smart albums or for preparing thumbnails. Both models need the shell to
  pass its library listing in (the shell does the decoding). Wiring them into `springboard/sdk/src/` is the next step.
- The favourites row (`Favorites`, status `os`) is not closed: favourite writes go through the shell's preference file
  and need an app-facing request first.
- The recently deleted rule is BifrOSt's (the shell's 30-day purge), not an Apple album rule.

## REA evidence

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/photokit (landing) | `ev_200a4547e2acb184dda2b384f2aefe270e2e804ba89fe56a8b4888ba55337185` |
| https://developer.apple.com/documentation/photokit/loading-and-caching-assets-and-thumbnails | `ev_5c6bf8fb00be610271b4a617fb4c7cb1f09592063fce7a355daf5e44906042b5` |
| https://developer.apple.com/documentation/photos/phassetcollectionsubtype | `ev_cd99904e2eb3fbf1fe1b9de4be54655cd9a50daaf06514067ed2f386264ce41b` |

DocC facts used (from `tools/parity/docc.py`): the caching manager's three start and stop calls and the
`allowsCachingHighQualityImages` property (`photos/phcachingimagemanager`); the three delivery mode constants
(`photos/phimagerequestoptionsdeliverymode`); the smart album subtype cases (`photos/phassetcollectionsubtype`).

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| PhotoKit | Smart albums (screenshots, recents, recently deleted) | partial | springboard/ui/src/photo_smart_albums.rs |
| PhotoKit | Image and thumbnail loading and caching | partial | springboard/ui/src/photo_caching.rs |
