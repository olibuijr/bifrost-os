# Wave 5, theme-11: index client delegate

Theme: "Index client delegate" (16 rows, 13 frameworks). This wave closes the Core Spotlight index-maintenance cluster.

## What was built

`springboard/ui/src/index_client.rs` (pure model, std only, registered in `springboard/ui/src/lib.rs`):

- `IndexDescription`: a named index with an item cap (1 to 100 000, default 1000 as in the shell index). Names are trimmed, must be non-empty and must not contain `/`. `storage_key(bundle_id)` gives `bundle/name`, so an app can keep several named indexes.
- `IndexStore`: the identifiers of one index. `index` is all or nothing: an empty identifier is `InvalidItem`, and a batch that would pass the cap is `QuotaExceeded`. Replacing an existing identifier does not use up the cap. `delete` and `delete_all` return the counts removed.
- `IndexError`: the eight codes of the official index error enumeration, with stable numeric codes and a `bifrost.index` domain.
- `IndexClientDelegate` trait: `reindex_all` and `reindex_items`. The result is the outcome reported back.
- `IndexExtension`: owns the app's index descriptions and runs a `Request` (`All` or `Items`) against the delegate. It always returns one `Acknowledgement`, which stands in for the completion handler. An unknown index is acknowledged as `IndexUnavailable` and the delegate is not called.

Tests: 7 in `#[cfg(test)] mod tests` (description validation, cap and replacement, all-or-nothing batch, delete counts, stable error codes, extension acknowledgement with delegate outcome, unknown index, item requests).

Not done: no SDK or shell wiring (no app trait method, no shell call to the extension), no protection classes, no data-for-item or file-URL requests, the store keeps identifiers only.

## Official API mirrored

- `CSSearchableIndexDelegate` (protocol): the delegate that the index calls to reindex. The Apple page did not render (see evidence below), so only the reindex-all role is taken from the extension page.
- `CSSearchableIndexDescription`: named index description. The official symbol page has no abstract in DocC, so the model is limited to name and item cap.
- `CSIndexExtensionRequestHandler`: "An interface that implements an index-maintenance app extension" and "use the reindex all method to reindex all items in your app".
- `CSIndexError` / `CSIndexError.Code`: `indexUnavailableError`, `indexingUnsupported`, `invalidClientStateError`, `invalidItemError`, `mismatchedClientState`, `quotaExceeded`, `remoteConnectionError`, `unknownError` (from DocC `csindexerror/code`).
- Article "Regenerating your app's indexes on demand": the system asks the app to regenerate its index when it is missing or out of date, and the delegate must call the acknowledgement handler.

## REA evidence

- `ev_3ee6bf44a68cad89a1aff26c590e5849c695ddf126ccea3d3abb8361c6a2c189`: `CSIndexExtensionRequestHandler` page (rendered; reindex-all text confirmed).
- `ev_e475cb6e4e38cb43cf8dab4d775374f5c611038eab34ceedbf5ca30b28457740`: "Regenerating your app's indexes on demand" article (rendered).
- `ev_c1943af43f1e93eed317e0790e2446f08402304b5e648106beb19a7e0ab85c04`: `CSSearchableIndexDelegate` page. The page rendered as "can't be found" (404), so it is not evidence of the protocol. DocC for `corespotlight/csearchableindexdelegate` and `corespotlight/csearchableindex` also returned 404 in this run.
- The Core Spotlight framework page evidence already in the matrix: `ev_4438bf59535e4bd6ba0ca70598c6a17e34d2cbccb6b6a1ca563a4e22a50cb41b`.

## Matrix rows closed

| framework slug | feature (as in data file) | new status | BifrOSt path |
|---|---|---|---|
| core-spotlight | Index client delegate | partial | springboard/ui/src/index_client.rs (IndexClientDelegate, IndexExtension) |
| core-spotlight | Index descriptions | partial | springboard/ui/src/index_client.rs (IndexDescription) |
| core-spotlight | Index extensions and on-demand regeneration | partial | springboard/ui/src/index_client.rs (IndexExtension, Request, Acknowledgement) |

Other theme rows (file-provider, matter-support, core-location, notification-center, core-foundation, media-library, core-animation, swiftdata, address-book, app-intents-testing, dispatch, endpoint-security) are not touched in this wave.

## Verification

Not built (wave rule). Checked by reading only: `python3 tools/check-names.py` exits 0 with no hits. The module uses std only, has no `unsafe`, and every test value was worked out by hand from the rules in the code.
