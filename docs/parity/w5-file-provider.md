# Wave 5: File Provider (placeholders and cloud search)

Two gap rows of the File Provider framework now have a pure-Rust model. Both extend the wave 1 modules
(`springboard/ui/src/file_provider.rs`, `springboard/sdk/src/file_provider.rs`); no new module was added.
The models are not wired into the Files app or the shell, and no extension host runs. Nothing was built or
run on the VM or on Midget.

## What was built

**Placeholders and download state (`springboard/ui/src/file_provider.rs`).** A provided item now carries
the content properties of the item protocol: `document_size` (`documentSize`), `content_modification_date`
(`contentModificationDate`, Unix seconds) and a `LocalContent` state that mirrors `isDownloaded` and
`isDownloading`: `Placeholder`, `Downloading`, `Downloaded`. Items are `Downloaded` by default. A provider
marks an item as a placeholder with `as_placeholder()`. The operations are gated by the item's
capabilities:

- `request_download()` needs `READING`. It moves a placeholder to `Downloading`; repeated requests change nothing.
- `finish_download()` moves `Downloading` to `Downloaded`. Any other state is `ItemError::InvalidState`.
- `evict()` needs `EVICTING`. It moves `Downloaded` to `Placeholder`, leaves a placeholder alone, and refuses
  a running download with `InvalidState`. The size and dates survive eviction.

`ItemError` gained the `InvalidState` variant.

**Cloud search (`springboard/sdk/src/file_provider.rs`).** The search protocol is modelled as data:

- `StringSearchRequest` mirrors the string search request: the trimmed `query` (blank is refused) and the
  `desired_number_of_results` hint, where zero means no limit.
- `SearchIndex` holds the items a provider hands it, per domain (`set_items` replaces one domain). It opts in
  with `with_string_search(true)`, the switch behind `supportsStringSearchRequest`. Without the opt-in,
  `search_enumerator` returns `None`.
- `search_enumerator` searches only domains that the `DomainRegistry` says are registered, enabled, visible
  and connected. The well-known containers (root, working set, trash) are never hits. Matching is a
  case-insensitive filename substring. The desired count caps the hits.
- `SearchEnumerator` hands out hits in batches with `next_batch(max)`. `is_finished()` is true once the hits
  are used up or the search was cancelled with `invalidate()`. A cancelled search returns no more hits.

Nothing is queried remotely. The search covers only what a provider has put into the index.

## Official API names mirrored

Official pages (read through `python3 -I tools/parity/docc.py`):

- `https://developer.apple.com/documentation/fileprovider` (framework landing page; lists the nonreplicated
  extension, the item protocol and the cloud search section)
- `https://developer.apple.com/documentation/fileprovider/nsfileprovideritemprotocol`: `documentSize`,
  `contentModificationDate`, `isDownloaded`, `isDownloading`, `capabilities`
- `https://developer.apple.com/documentation/fileprovider/nsfileprovidersearching`: the search protocol,
  with `supportsStringSearchRequest` and `searchEnumerator(for:)`
- `https://developer.apple.com/documentation/fileprovider/nsfileproviderstringsearchrequest`: `query`,
  `desiredNumberOfResults`
- `https://developer.apple.com/documentation/fileprovider/nsfileprovidersearchenumerator`:
  `enumerateSearchResults(for:startingAt:)`, `invalidate()`

The Rust names are snake_case and the Apple selectors are not claimed. The owner rule keeps Apple type
names out of the code.

## REA evidence

- Framework landing page: `ev_fceffe45bcc6ffb33807bb22eb0eabffca82b5e5ccd8839a2cff3552b1680f06`
- NSFileProviderSearching page: `ev_fbaa48e50e18e9cf6460436635f44864a9e7578612fd5e61cb98cc98c40a5444`

## Matrix rows closed

These rows are from `docs/parity-matrix/data/file-provider.json`. Their new status is `partial`. The lead
updates the matrix centrally.

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| file-provider | Nonreplicated File Provider extension (managed local files with placeholders) | partial | springboard/ui/src/file_provider.rs |
| file-provider | Cloud search (NSFileProviderSearching) | partial | springboard/sdk/src/file_provider.rs |

## Still missing

- No extension host, no IPC, and no file system placeholders on disk. The Files app still sees only
  ordinary files (`springboard/src/files/fs.rs`).
- No download or upload progress, no download error, and no upload state.
- The search has no remote query and no result observer callbacks.
- The `FSEventStream` change feed (row "Change feed for item updates") is not connected to the domain
  signals. It was not touched in this wave.
