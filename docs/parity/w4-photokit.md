# PhotoKit parity, wave 4

Official reference: https://developer.apple.com/documentation/photokit (landing page). The framework page is
https://developer.apple.com/documentation/photos. The DocC subpaths for the individual classes returned 404 from
`tools/parity/docc.py`, so the API names below come from the rendered framework pages only.

REA evidence:
- `ev_be2eb4021f9e557fd31ea882d36f787f8361b5c6b015df50b13c7cdcf01545be` (PhotoKit landing page)
- `ev_090cf246f56c26aa90a2a46a7e4e0460e9d5d7ae48ae5046bfd78e854b8fdc1d` (Photos framework page)

## What was built

Two pure models in `springboard/ui/src/` (`bifrost_ui`). Both are std only and have no shell or file access.

### `photo_fetch.rs`: fetch options for a library listing

Mirrors the filtering and ordering that the reference's fetch options control, over a listing of `Item` values.

- `FetchOptions` keeps items by media kind, favourite flag, and creation-time range (`from` inclusive, `to`
  exclusive). It sorts newest or oldest first (`Sort`) and applies an optional `limit`. Trashed items are dropped
  unless `include_trashed` is set.
- Items with the same creation time are ordered by identifier, so a query always returns the same order.
- `fetch` returns references into the input slice. `count` returns the number of matches.
- Mirrored names: PHFetchOptions (filtering and sorting), PHFetchResult (the ordered result), PHAsset (`Item`).

### `photo_access.rs`: authorization state and limited access

Mirrors the shared library access flow and the limited, person-chosen access scope.

- `Status` covers not determined, restricted, denied, authorized and limited. `Access::request` takes an `Answer`
  (allow, allow limited to chosen identifiers, deny). Only the first request changes the state. A restricted library
  refuses every request.
- In limited access, `select` replaces the chosen identifiers. `allows` answers whether one item is visible.
- Mirrored names: PHPhotoLibrary authorization and the prompt, and the limited-library scope from the enhanced privacy
  article.

Both modules are registered in `springboard/ui/src/lib.rs`, after `photo_request`.

## Not done

- No SDK wrapper in `springboard/sdk/src/` yet. Apps cannot reach either model until one exists, so the `os` row is
  closed as `partial`, not `parity`.
- The shell library (`springboard/src/shell/photos/`) does not use these models yet. Nothing reads or writes the
  Photos library through them.
- No cargo build and no test run were done, per the wave rules. The tests were written by hand from the rules the
  code implements, but they have not been executed.

## Matrix rows closed

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| PhotoKit | Asset retrieval and fetch options | partial | springboard/ui/src/photo_fetch.rs |
| PhotoKit | Shared photo library access (PHPhotoLibrary) | partial | springboard/ui/src/photo_access.rs |
| PhotoKit | Enhanced privacy and limited library access | partial | springboard/ui/src/photo_access.rs |

## Tests

- `photo_fetch.rs`: 7 tests (default filter and order, media kind, favourites with oldest first, date range bounds,
  limit, trashed items with id tie-break, empty library).
- `photo_access.rs`: 6 tests (first allow, repeated request keeps the first answer, limited access with duplicate
  identifiers, selection only in limited access, restricted library, denied and undecided apps).

## Checks

- `python3 tools/check-names.py` exits 0 with no hits.
