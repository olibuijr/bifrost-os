# Swift framework parity (Synchronization, Regex)

Matrix source: `docs/parity-matrix/data/swift.json`. Framework status stays `partial`: most of the Swift docs are
the language itself, which BifrOSt apps (Rust) do not expose.

## What was built

| Area | Official API mirrored | BifrOSt location | Status |
|---|---|---|---|
| Atomic values | `Atomic`, `AtomicLoadOrdering`, `AtomicStoreOrdering`, `AtomicUpdateOrdering`, `add(_:ordering:)`, `subtract`, `bitwiseAnd`, `bitwiseOr`, `bitwiseXor`, `exchange`, `compareExchange` | `springboard/sdk/src/synchronization.rs` (`bifrost_app::synchronization`) | partial |
| Mutual exclusion | `Mutex`, `withLock(_:)`, `withLockIfAvailable(_:)` | same file (`Mutex::with_lock`, `Mutex::with_lock_if_available`) | partial |
| Memory fence | `atomicMemoryFence(ordering:)` | same file (`atomic_memory_fence`) | partial |
| Regex | `Regex`, `Regex.Match`, `init(_:)`, `init(verbatim:)`, `ignoresCase(_:)`, `dotMatchesNewlines(_:)`, `firstMatch(in:)`, `prefixMatch(in:)`, `wholeMatch(in:)` | `springboard/ui/src/regex.rs` (`bifrost_ui::regex`, also `bifrost_app::controls::regex`) | partial |

Design notes:

- `Atomic<T>` wraps std atomics through the `AtomicRepresentable` trait, implemented for `bool`, `i32`, `i64`,
  `isize`, `u32`, `u64` and `usize`. The ordering enums mirror the framework's three kinds, so a load cannot take a
  release ordering. `compare_exchange` returns `Result<T, T>` (`Ok` with the expected value, `Err` with the current one).
- `Mutex<T>` ignores poisoning, as the framework's lock does. `with_lock_if_available` never waits.
- `Regex` is a backtracking matcher over characters with byte-offset ranges. Supported: literals and escapes, `.`,
  `^` and `$` (whole-input anchors), `[...]` and `[^...]` classes, `\d \w \s` and negations (ASCII), `( )` and
  `(?: )`, `|`, `* + ? {n} {n,} {n,m}`, and lazy quantifiers with a trailing `?`. An empty iteration is not repeated,
  so `(a*)*` terminates. `Regex::new` returns a `RegexError` with a pattern position.

## Official documentation and REA evidence

- Synchronization index: https://developer.apple.com/documentation/synchronization
  REA evidence `ev_9dab53bee08615ec998db9afdbe460d6f67ba76e3236c4bafc6dba44342bf42a` (rendered page).
  `Mutex`: `withLock(_:)`, `withLockIfAvailable(_:)`. `Atomic`: `add(_:ordering:)`, `bitwiseAnd`, `bitwiseOr`,
  `bitwiseXor`, `subtract` and the ordering parameters (read with `docc.py synchronization/mutex` and
  `docc.py synchronization/atomic`).
- Regex: https://developer.apple.com/documentation/swift/regex
  REA evidence `ev_4f5f48e60ee13ebaa3518475dbd7fa63066c4ee6e50b618d2a188e09a3752e02` (rendered page). Initializers
  `init(_:)`, `init(verbatim:)`, `ignoresCase(_:)`, `dotMatchesNewlines(_:)`, `firstMatch(in:)`, `prefixMatch(in:)`,
  `wholeMatch(in:)`.

## Remaining gaps

- Synchronization: `AtomicLazyReference`, `WordPair`, `AtomicOptionalRepresentable`, `Atomic` min and max
  operations, and the `AtomicRepresentable` protocol for app-defined types.
- Regex: `RegexBuilder` (the declarative result-builder DSL), typed and named captures (`Regex.Match.output` is the
  whole match only), `matchingSemantics`, `repetitionBehavior`, `wordBoundaryKind`, `anchorsMatchLineEndings`, Unicode
  shorthand classes (the `asciiOnly*` options are not exposed), and the `Regex` output-type generics.
- Still `gap` in the matrix: actors and isolation, distributed actors, and the Codable `CodingKeys` renaming.

## Verification

Not built or tested in this change: the build and test stage runs `cargo test` on `bifrost-ui` and `bifrost-app`.
Unit tests live in `springboard/sdk/src/synchronization.rs` (8 tests) and `springboard/ui/src/regex.rs` (11 tests).
