# Wave 4: Background Assets

Framework: Background Assets (https://developer.apple.com/documentation/backgroundassets), since macOS 13.0.

This wave closes three gap or partial rows of the matrix with pure Rust models. The code has no I/O: a test source
stands in for the download host, and the manifest reads and writes text lines.

## What was built

### `springboard/ui/src/assetpack.rs` (extended)

- `LocalAvailabilityError { successes, failures }`, returned by `AssetPackManager::ensure_local_availability(ids)`.
  Each requested id is either a success (available at its current version) or a failure with a reason. Unknown ids
  fail with `PackError::UnknownPack`. Missing and outdated packs fail with the new `PackError::NotAvailable`.
  Both lists keep request order. Apple's `successes` is a `Set` and `failures` a dictionary; the order here is
  deterministic.
- `PackError::Malformed(line)`, for manifest lines that cannot be read.
- `AssetPackManifest::to_lines()` and `AssetPackManifest::from_lines(text)`: one tab-separated line per pack (id,
  version, download bytes, file sizes separated by commas, languages separated by commas). `from_lines` applies the
  same id checks as `new`. This is a plain text form, not Apple's JSON manifest: `init(from:appGroupID:)` and
  `init(contentsOf:appGroupID:)` are not mirrored, and no app group id is read.
- Tests: `local_availability_splits_successes_from_reasons`, `manifest_lines_round_trip_files_and_languages`,
  `manifest_lines_reject_bad_fields_with_their_line_number`.

### `springboard/ui/src/assetmock.rs` (new)

`MockPackSource` is an in-memory stand-in for the pack host used in tests (`Testing asset packs locally`):

- `add_fixture(id, bytes)` stores the bytes served for a pack. The bytes must be as long as the pack's download size.
- `set_reachable(false)` makes every fetch fail with `MockError::Unreachable`.
- `fetch(id)` returns the stored bytes, or `NoFixture`.
- `deliver(manager, id)` fetches a pack and marks it installed in an `AssetPackManager`.

It opens no sockets and uses no HTTPS, certificates or URL override. Apple's mock server is an HTTPS process on a
developer Mac, so only the local fixture flow is mirrored. Tests: `fixture_must_match_the_download_size`,
`fetch_without_a_fixture_names_the_pack`, `unreachable_source_fails_even_with_a_fixture`,
`deliver_installs_the_pack_in_the_manager`, `deliver_leaves_the_manager_alone_when_the_fetch_fails`,
`deliver_reports_a_pack_the_manager_does_not_know`.

## Official API names mirrored

| Apple name | BifrOSt |
|---|---|
| `AssetPackManager.LocalAvailabilityError` (`successes`, `failures`) | `LocalAvailabilityError`, `ensure_local_availability` |
| `ManagedBackgroundAssetsError.assetPackNotFound(withID:)` | `PackError::UnknownPack` (existing) |
| `AssetPackManifest` (`init(from:appGroupID:)`, `assetPacks`, `assetPack(withID:)`) | `AssetPackManifest::from_lines`, `to_lines` (text form; `asset_pack` existed) |
| Testing asset packs locally (mock server) | `MockPackSource` |

Not built: `ManagedDownloaderExtension`, `BADownloaderExtension`, `BAManifestURL`, `BAAppGroupID`,
`BAErrorCode` values, `BAErrorDomain`. These need the system scheduler or an extension host.

## REA evidence

- `ev_2d91bd9b8b9a617aa8a02a94ba4fd043a09ec5b8343e41d14e3547a5698b8620`: Background Assets landing page
- `ev_d073d1eff4f70be34d1be7fc0731f73bcae3986321a0aea9eed6ff8e6db721dc`: Testing asset packs locally
- `ev_af31634c533f62c4752419a671a57f98b9f1b95ac17454540020df73c8d75def`: AssetPackManager.LocalAvailabilityError

## Matrix rows closed

Framework slug `background-assets`. Features are quoted exactly as in `docs/parity-matrix/data/background-assets.json`.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Background Assets | Errors (ManagedBackgroundAssetsError, BAErrorCode, LocalAvailabilityError) | partial | springboard/ui/src/assetpack.rs (`LocalAvailabilityError`, `PackError::NotAvailable`) |
| Background Assets | Downloader extension and manifest (BADownloaderExtension, BADownloaderExtensionConfiguration, BAManifestURL) | partial | springboard/ui/src/assetpack.rs (`AssetPackManifest::to_lines`, `from_lines`) |
| Background Assets | Testing packs against a local mock server | partial | springboard/ui/src/assetmock.rs (`MockPackSource`) |

## Notes for the lead

- The matrix note on `Unmanaged download manager and URL downloads` says there is no persistent queue and no
  essential/non-essential priority. `springboard/ui/src/downloadqueue.rs` already has both (`Priority`, `Budget`,
  `to_lines`/`from_lines`). That note is stale and was not changed here.
- `Downloader extension and manifest` stays partial: no extension runs while the app is closed.
