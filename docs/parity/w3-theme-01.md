# Wave 3, theme 01: iCloud / ubiquitous data

## What was built

`springboard/ui/src/ubiquitous_store.rs` (`bifrost_ui::ubiquitous_store`), a pure model of a small key-value store
synced across one person's devices. It covers the key-value half of the theme only. There is no transport, no
account service and no document storage, so the document half of the theme stays a gap.

- `Value`: bool, double, integer, text and byte data. Each value has a size that counts against the quota.
- `UbiquitousStore`: local replica for one device. `set`, `get`, `remove` and `keys` work on live keys.
  `set` and `remove` queue pending changes, and `synchronize` drains them as `Change` records with their stamp
  and origin device.
- `apply_remote` and `apply_initial`: last writer wins on the pair (logical stamp, origin device name), so the
  result does not depend on the order changes arrive in. A duplicate change is a no-op. A removal is a tombstone.
  An accepted remote change clears any local pending write for the key. The local clock moves past remote stamps.
- Quota: a byte budget (key length plus value size, over live keys) and a live-key limit, both set by the caller.
  A write that breaks either limit is refused and records `QuotaViolationChange`.
- `take_changes`: returns the reason (`ServerChange`, `InitialSyncChange`, `QuotaViolationChange`,
  `AccountChange`) and the sorted keys changed by remote data, then clears them. This mirrors the change
  notification with its change-reason key.

## Official API mirrored

Apple reference: `NSUbiquitousKeyValueStore` (https://developer.apple.com/documentation/foundation/nsubiquitouskeyvaluestore),
with `set(_:forKey:)`, the typed getters, `removeObject(forKey:)`, `synchronize()`, the did-change-externally
notification, and the change reason constants (server change, initial sync, quota violation, account change).
Official rule taken from the page: when a write would exceed the quota, the system adds nothing and posts a
quota-violation change, which the model matches.

## Rows that are not closed

- Quota numbers: the docs page lists the size limits but they were not visible in the rendered text read here, so
  the store takes the budget and key count from the caller and does not hard-code Apple's values.
- No `Date`, array or dictionary values; the reference page says to archive other types first.
- No durable write-behind (the reference store writes to disk asynchronously; this model is memory only).
- CloudKit and SwiftData sync (record and model sync) are untouched by this change.

## REA evidence

- `ev_c71bdcc03a95318006a6d6d5d580df4838c784ba7f601960352669d6e8ba3320`: REA inspect of
  https://developer.apple.com/documentation/foundation/nsubiquitouskeyvaluestore (title "NSUbiquitousKeyValueStore").

## Matrix rows closed (status partial, proposed for the lead to update centrally)

| framework slug | feature (as in data json) | status | BifrOSt path |
|---|---|---|---|
| appkit-app-structure | iCloud / ubiquitous data | partial | springboard/ui/src/ubiquitous_store.rs (key-value part only; no document storage) |
| foundation-files-persistence | iCloud documents and key-value store | partial | springboard/ui/src/ubiquitous_store.rs (key-value part only; no ubiquity containers) |

## Tests

`cargo test` was not run (no builds in this round). Nine tests sit in the module's `tests` block; their expected
values were worked out by hand from the rule each test checks (sizes, stamps, quota arithmetic).
