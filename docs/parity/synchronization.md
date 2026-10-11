# Synchronization parity

Official reference: https://developer.apple.com/documentation/synchronization (macOS 15.0+).

## What was built

Module `springboard/sdk/src/synchronization.rs`, exposed to apps as `bifrost_app::synchronization`. Pure std: no
new crates, no `unsafe`, no hardware or network. Each type maps onto `std::sync` as follows.

| Official API | BifrOSt | Notes |
|---|---|---|
| `Mutex<Value>`, `init`, `withLock(_:)`, `withLockIfAvailable(_:)` | `Mutex<T>::new`, `with_lock`, `with_lock_if_available` | Closure-scoped lock; the `_if_available` form returns `Option<R>` and never blocks. A poisoned lock is recovered. |
| `AtomicLoadOrdering`, `AtomicStoreOrdering`, `AtomicUpdateOrdering` | `LoadOrdering`, `StoreOrdering`, `UpdateOrdering` | Same case sets; mapped to `std::sync::atomic::Ordering`. |
| `atomicMemoryFence(ordering:)` | `atomic_memory_fence(UpdateOrdering)` | `Relaxed` is a no-op (std rejects a relaxed fence). |
| `Atomic<Value>`: `init`, `load`, `store`, `exchange`, `compareExchange` (both forms), `add`, `subtract`, `bitwiseAnd`, `bitwiseOr`, `bitwiseXor` | `Atomic<T>` with `new`, `load`, `store`, `exchange`, `compare_exchange`, `compare_exchange_with`, `add`, `subtract`, `bitwise_and`, `bitwise_or`, `bitwise_xor` | Over `u32`, `u64`, `usize`, `i32`, `i64`, `isize`. Read-modify-write operations return `(old, new)`; `compare_exchange` returns `(exchanged, found)`. Wrapping arithmetic. |
| `AtomicLazyReference`: `init`, `load()`, `storeIfNil(_:)` | `AtomicLazyReference<T>::new`, `load`, `store_if_nil` | Built on `OnceLock<Arc<T>>`. The first stored value wins; `store_if_nil` returns the stored reference. |

The `storeIfNil(_:)` abstract is truncated in the DocC output, so the "already set" behaviour is taken from its
opening sentence (it returns the value already held) and is checked by a unit test.

## Tests

`#[cfg(test)] mod tests` in `synchronization.rs`: closure-scoped mutation, `with_lock_if_available` while held,
poisoned-lock recovery, threaded mutex increments, wrapping add and old/new reporting, bitwise operations, exchange
and compare-exchange, threaded atomic counting, fence over every ordering, and first-store-wins for the lazy
reference. None was built or run in this round (the stage that builds and tests the workspace runs them).

## REA evidence

- Framework landing page: `ev_21aafe50be861f8ae16bf16388450c0c9984b2d36a2071d63bf34fe8418fcf55`
  (captured with `tools/parity/rea-page.sh https://developer.apple.com/documentation/synchronization`).
- The earlier framework evidence `ev_6fea1ac9f2e1722b2413b00336b9d3d5a18d4d447fa76ead52cd0a7f93c6b4a9` is recorded
  in the previous matrix row and is superseded for the landing page.

## Remaining gaps

- `WordPair` (a pair of two word-sized integers stored atomically). A lock-free double-word compare-and-exchange
  needs `unsafe` or a platform instruction, which this round excludes. Not built.
- `AtomicRepresentable` and `AtomicOptionalRepresentable`. `Atomic` covers six fixed-width integer types only, not
  `Bool`, `Optional` or custom types.
- `AtomicLazyReference` holds `Arc<T>`. There is no equivalent of the object-reference model beyond `Arc`.

Matrix row status: Mutex, the orderings and the fence are `parity`; Atomic and AtomicLazyReference are `partial`;
WordPair is `gap`; the representable protocols are `n/a`. Framework status stays `partial`.
