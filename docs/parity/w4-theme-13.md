# Wave 4: theme-13, downloading and compiling a model at runtime

Three gap rows of the Core ML and Core AI matrices are now partial. The theme's remaining rows (ahead-of-time
compilation, custom text models, language model providers, model-judge evaluators) stay gaps, see "Not done".
Pure Rust, std only, with no hardware, no network and no unsafe code.

## What was built

- `springboard/ui/src/model_assets.rs` (bifrost-ui): an `AssetCatalog` of named model assets. Each asset moves
  through `begin_download` (target version and byte total), `progress`, `finish_download` (with a digest of the
  bytes), `begin_compile`, `finish_compile` and `fail`. The ready version and the target version are kept apart:
  a newer version that is downloading, compiling or failed leaves the ready version usable. A version that is not
  newer than the ready one is refused (`NotNewer`), and a second version cannot start while one is in flight
  (`Busy`). Every refused step returns an error and leaves the state unchanged. Nothing fetches, compiles or reads a file.
- `springboard/ui/src/model_specialization.rs` (bifrost-ui): a `SpecializationCache` of prepared model artifacts,
  keyed by model name and specialization options. Each entry has a `Policy`: `Purgeable` entries share the existing
  cost-limited LRU (`springboard/ui/src/cache.rs`) and go first, by the limit or by `purge`; `Persistent` entries
  are never evicted by the limit or a purge and leave only by an explicit delete. It also offers `delete_entry`,
  `delete_entries_for` (one model, all options) and `delete_all`. The artifact is opaque bytes from the caller.
- `springboard/ui/src/cache.rs`: extended with `contains_key`, which does not refresh recency. The specialization
  cache uses it to keep its key index equal to the LRU's contents. One test covers it.
- `springboard/ui/src/lib.rs`: one `pub mod` line for each new module.

Both new modules reuse the existing `cache::Cache`; no LRU was duplicated. `model_store` and `model_history` are record
stores for app data, so they were not reused for model files.

## Official API mirrored

- `MLModelCollection` and the on-device download and compile flow (Downloading and Compiling a Model on the User's
  Device). The app obtains a model, compiles it on the device, and keeps the result in a stable location. The
  `Phase` and `AssetCatalog` types mirror that flow as a state machine. Sources:
  https://developer.apple.com/documentation/coreml/mlmodelcollection (REA evidence
  `ev_cb11acafc237193f7e11a8536e27fe816c97cad31bb4833d064aaa7e3cf949dc`) and
  https://developer.apple.com/documentation/coreml/downloading-and-compiling-a-model-on-the-user-s-device (REA evidence
  `ev_721185d4b57d5db0e58af6d519bd91245c5c6a5daa61d8bcd995a31308160fb4`).
  The official page describes an Apple-hosted deployment service. BifrOSt has none, so the catalog takes the bytes and
  the compile result from the caller and does not describe where they come from.
- `AIModelCache` (Core AI). The cache holds the specialized, device-specific artifacts that an inference model loads.
  Entries are keyed by a model and its specialization options. `Policy` mirrors `AIModelCache.Policy` (`default`
  marks entries purgeable, `persistent` prevents purging). `delete_entries_for` mirrors `deleteEntries(for:)`,
  `delete_entry` mirrors `deleteEntry(for:options:)`, and `delete_all` mirrors `deleteAll()`. Source:
  https://developer.apple.com/documentation/coreai/aimodelcache (REA evidence
  `ev_5371c19a4158ced461573831eee6d4bc2ca5cd84163356c61d0663cd2523e913`). The policy page
  (https://developer.apple.com/documentation/coreai/aimodelcache/policy) and the article
  https://developer.apple.com/documentation/coreai/managing-model-specialization-and-caching were read from DocC
  only; no REA capture was taken for them. Purge conditions (`PurgeConditions`) are not modelled.
  The app-group shared cache (`init(appGroup:)`) and bookmark-based deletion are not modelled: there is no shared
  container on this platform.
- `AIModelAsset` (the unspecialized source asset in Core AI's "Integrating on-device AI models in your app" flow).
  `AssetCatalog` tracks the source asset's versions and readiness. It does not load the asset or run inference.

## Matrix rows closed

| Framework | Feature (as written in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| core-ml | Downloading and compiling a model at runtime | partial | springboard/ui/src/model_assets.rs |
| core-ai | Model cache and specialization | partial | springboard/ui/src/model_specialization.rs |
| core-ai | Integrating on-device models | partial | springboard/ui/src/model_assets.rs |

Status is `partial`, not `parity`, because nothing here is wired into an app or the shell and the code is not run.
Per the owner rule, a row becomes `parity` only after the feature works on the VM and on Midget.

## Tests

- `model_assets.rs`: six tests (full cycle to ready, byte-count checks, out-of-order steps refused, an update keeps
  the ready version, one version in flight at a time, a ready asset cannot fail and removal is reported).
- `model_specialization.rs`: seven tests (lookup by model and options, LRU eviction, persistent entries survive the
  limit and a purge, delete for one model, oversize artifact not kept and not indexed, persistent store replaces a
  purgeable entry without double counting, delete all).
- `cache.rs`: `contains_key_does_not_refresh_recency`.

The expected values were worked out by hand from the rules the code implements. Nothing was compiled or run in this
wave, per the wave rules, so the tests are written, not executed.

## Not done

- No compile step. The compile result is supplied by the caller, and BifrOSt has no compiler for model packages.
  Ahead-of-time compilation (`coreai-build`) stays a gap, and so do the compile-related parts of Core ML.
- No download transport. The catalog never fetches bytes. No deployment service, no collection updates
  (`didChangeNotification`), no model encryption, no custom layers and no on-device updates (`MLUpdateTask`).
- No model execution. `AIModel` and inference binding are not part of this wave; `Integrating on-device models`
  is partial only for the asset half.
- Custom text models (NLModel), contextual embeddings, the language model provider and model-judge evaluators are
  not part of this theme's closed rows and stay gaps.
