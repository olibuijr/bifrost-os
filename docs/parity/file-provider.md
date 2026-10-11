# File provider parity: domains, provided items, errors and enumerator signals

Official reference: <https://developer.apple.com/documentation/fileprovider> (File Provider, macOS 10.15 and later
for the framework, the individual symbols from macOS 11.0). Matrix row: `docs/parity-matrix/data/file-provider.json`.

## What was built

Pure Rust with std only. No extension process, no IPC, no hardware and no network.

- `springboard/ui/src/file_provider.rs` (`bifrost_ui::file_provider`), the item model:
  - `ItemIdentifier`: a non-empty persistent identifier, with `root_container()`, `working_set()` and
    `trash_container()`.
  - `ItemCapabilities`: a bit set with the ten `allows*` flags, `ALL` (allowsAll), `contains`, `union` and `with`.
  - `TypeAndCreator`: two four-character printable codes, with the type packed as a big-endian OSType.
  - `ProvidedItem`: identifier, parent, validated filename, content type, type and creator, and the capability
    set. `rename` needs `RENAMING` and `reparent` needs `REPARENTING`. Names with `/`, NUL, `.` or `..` are refused.
- `springboard/sdk/src/file_provider.rs` (`bifrost_app::file_provider`), the app-facing model:
  - `ErrorCode`: 19 official error codes, with `as_str` (official case names), `retry_later` and
    `restarts_enumeration`.
  - `ProviderError`: a code and an optional item identifier, with `Display` and `std::error::Error`.
  - `Domain`: identifier, display name, storage path (relative, no `..`), replicated, hidden, user enabled and
    disconnected state.
  - `DomainRegistry`: `add` (unique identifiers), `remove` (also drops that domain's pending signals), `all`,
    `get`, `visible`, `set_user_enabled`, `disconnect`, `reconnect`, `signal_enumerator` and `take_signals`.
    Signals for the same container coalesce and carry the newest generation. A disconnected domain refuses a
    signal with `providerDomainTemporarilyUnavailable`.
- `springboard/sdk/src/lib.rs` and `springboard/ui/src/lib.rs` each gained one `pub mod file_provider;` line.

Tests: 6 in `springboard/ui/src/file_provider.rs` and 7 in `springboard/sdk/src/file_provider.rs`. They have not
been run in this round, because the build and test stage runs them.

## Official API names mirrored

| Official name | BifrOSt |
|---|---|
| `NSFileProviderItemIdentifier` (`rootContainer`, `workingSet`, `trashContainer`) | `ItemIdentifier` and its constructors |
| `NSFileProviderItemCapabilities` and its `allows*` values, `allowsAll` | `ItemCapabilities` and its constants |
| `NSFileProviderTypeAndCreator` | `TypeAndCreator` |
| `NSFileProviderItemProtocol` (identifier, parentIdentifier, filename) | `ProvidedItem` |
| `NSFileProviderDomain` (identifier, displayName, pathRelativeToDocumentStorage, isReplicated, isHidden, userEnabled, isDisconnected) | `Domain` |
| `NSFileProviderManager` (add, remove, getDomains, disconnect, reconnect, signalEnumerator) | `DomainRegistry` |
| `NSFileProviderError.Code` | `ErrorCode` |
| `NSFileProviderError` | `ProviderError` |

## REA and documentation evidence

- REA evidence of the rendered landing page (this round): `ev_c9d3cec28f780836bffdd407ba62cfa3f732fdc01ccc27abf59e4fdde25b0703`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/fileprovider`, title "File Provider").
- REA evidence recorded in the matrix before this round: `ev_f8364b6e41237dff7d334042383d3cdde778a88859b03c486cab46a0b831226f`.
- Official API shape came from `tools/parity/docc.py` on `fileprovider`, `fileprovider/nsfileprovideritemcapabilities`,
  `fileprovider/nsfileproviderdomain`, `fileprovider/nsfileprovidererror/code`, `fileprovider/nsfileprovidermanager`,
  `fileprovider/nsfileprovideritemidentifier` and `fileprovider/nsfileprovidervolumeunsupportedreason`.

## Matrix rows changed

| Row | Before | After |
|---|---|---|
| Extension management | gap | partial (signals only) |
| Provided items | partial | partial (model, no versions or sync state) |
| Domains | gap | partial (registry, no volume eligibility) |
| Errors | partial | partial (official codes, no user-info keys) |

The framework status moved from gap to partial, because it now has counterparts for several features.

## Remaining gaps

- No provider extension host, so no replicated or nonreplicated extension and no way for an app to register a provider.
- Enumerator signals are queued but nothing reads them. There is no reimport, `waitForChanges`, or materialized or
  pending item enumerator.
- No placeholder (dataless) files and no on-demand hydration in Files or the document panel.
- No provider search (`NSFileProviderSearching` equivalent).
- No volume eligibility (`NSFileProviderVolumeUnsupportedReason`), no known-folder replication, no testing modes,
  no download or upload progress.
- Coordination is advisory, with no save negotiation, relinquish or versions (see `docs/bifrost-ui.md`).
