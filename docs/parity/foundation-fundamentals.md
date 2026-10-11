# Foundation (Fundamentals): parity work

Framework: Foundation, fundamentals section. Official docs: https://developer.apple.com/documentation/foundation
Matrix row: `docs/parity-matrix/data/foundation-fundamentals.json`.

## What was built

Two pure-Rust, std-only additions. Neither uses unsafe code, a new crate, hardware or a network service.

1. `springboard/ui/src/indexset.rs` (bifrost-ui, module `indexset`): `IndexSet`, a set of integers stored as sorted,
   disjoint, non-touching ranges. It covers insert and remove (single integer and range), `contains`,
   `contains(integersIn:)`, `intersects(integersIn:)`, `count(in:)`, `count`, `first`, `last`, the range view,
   iteration, the less-than and greater-than neighbour lookups, `formUnion`, `formIntersection`,
   `formSymmetricDifference` with their non-mutating forms, and `filteredIndexSet(in:includeInteger:)`.
   It has six behaviour tests.
2. `springboard/core/src/progress.rs` (core, module `progress`): `ProgressTree`, a tree of work jobs in one arena,
   named by `ProgressId`. A root job has a unit total. A child job takes a pending share of its parent's units. The
   fraction completed is the job's finished units plus each child's pending share times that child's fraction,
   clamped to 1. It also covers finish, advance, set total, `is_finished`, and cancel. Cancel reaches every
   descendant, and a child added after cancel starts cancelled. It has six behaviour tests.

Module registration: one `pub mod indexset;` line in `springboard/ui/src/lib.rs` and one `pub mod progress;` line in
`springboard/core/src/lib.rs`.

## Official API names mirrored

- `IndexSet`: `init(integer:)`, `init(integersIn:)`, `insert(_:)`, `insert(integersIn:)`, `remove(_:)`,
  `remove(integersIn:)`, `removeAll()`, `contains(_:)`, `contains(integersIn:)`, `intersects(integersIn:)`,
  `count(in:)`, `count`, `first`, `last`, `rangeView(of:)`, `integerLessThanOrEqualTo(_:)`,
  `integerGreaterThan(_:)`, `integerGreaterThanOrEqualTo(_:)`, `integerLessThan(_:)`, `formUnion(_:)`,
  `formIntersection(_:)`, `formSymmetricDifference(_:)`, `filteredIndexSet(in:includeInteger:)`.
  Source: https://developer.apple.com/documentation/foundation/indexset (checked with `tools/parity/docc.py`).
- `ProgressTree`: `init(totalUnitCount:)`, `init(totalUnitCount:parent:pendingUnitCount:)`, `totalUnitCount`,
  `completedUnitCount`, `fractionCompleted`, `isFinished`, `cancel()`, `isCancelled`.
  Source: https://developer.apple.com/documentation/foundation/progress (checked with `tools/parity/docc.py`).

Rust names differ where the Apple name would be ambiguous in Rust. For example, `integer_less_than_or_equal_to`
keeps the Apple name in snake case, and `ranges()` stands in for `rangeView`.

## REA evidence

- `ev_b1da1450ee8a71ad02f6ff0ad02cc6e2fbf8e5ffe09735813b412e818b7ba4bd`: rendered Foundation landing page
  (`https://developer.apple.com/documentation/foundation`). It lists Progress Manager, Progress Reporter and
  Subprogress under Task Management and the Fundamentals topics, and NSPredicate Validating under Protocols.
- `ev_f121193127e3dd8357738ded0ca4fc3e8cf37079a4700254b73fb29165bc71e2`: the earlier landing capture recorded in the
  matrix before this change. It stays as the prior record.

## Remaining gaps

- No app-facing SDK wrapper for progress trees. Apps cannot create a progress job yet, and the tree is not wired to
  the download, install or file-copy paths.
- No `ProgressReporter` protocol, no pause or resume, no time estimates, no throughput, no file-operation or
  cross-process publishing.
- A job with no units reports 0 until it has some. A finished child with zero units does not reach its parent.
- `IndexSet` has no shifting (`shift(startingAt:by:)`), no `IndexSet.Index` type, and no `RangeView` struct. It
  exposes runs as a slice instead.
- An integer must stay below `usize::MAX`, because a single integer is stored as `i..i + 1`.
- No predicate or expression engine, no regular expression or natural-language API, no time zones, no ordered or
  counted sets, no Decimal, UUID or NSNumber wrappers. These remain the matrix gaps.

## Verification

Not built or tested in this stage, as the task requires. The owner name check passes:
`python3 tools/check-names.py` exits 0. Another stage runs `cargo test` for `bifrost-ui` and `core`.
