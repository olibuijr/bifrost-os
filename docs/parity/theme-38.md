# Theme 38: boundary entries, activity entries, payload entries and enumerator positions

Official reference: the OSLog framework, https://developer.apple.com/documentation/oslog (landing page with the
Log Entries and Entry Data topics). REA evidence of the rendered page: `ev_940397b0eeb9d9fa961191ed0517b9fcdc081826bcc990126aaaa615149bec6f`.
The DocC abstracts checked with `tools/parity/docc.py` were `OSLogEntryBoundary` ("The metadata that partitions
sequences of other entries"), `OSLogEntryActivity` (with `parentActivityIdentifier`), `OSLogPosition` ("A
representation of a point in a sequence of entries"), `OSLogEnumerator` (with `Options`, the direction option),
`OSLogEntryWithPayload` (`formatString`, `components`, `subsystem`, `category`) and `OSLogMessageComponent`
(`formatSubstring`, `placeholder`, `argumentCategory`, and the `argument*Value` accessors).

The abstract-level DocC output does not list every case of `OSLogMessageComponent.ArgumentCategory` or the fields of
`OSLogEntryBoundary`, so those parts are this project's own model and are marked as such below.

## What was built

Two pure std-only modules in `springboard/core/src/` (re-exported through the `bifrost_core` crate, no new crates,
no unsafe, no I/O). Both are registered in `springboard/core/src/lib.rs`.

### `springboard/core/src/log_entries.rs`

- `Entry`: one item of a sequence, `Log(LogEntry)`, `Payload(Payload)`, `Activity(Activity)` or `Boundary(Boundary)`,
  with `date()` and `From<LogEntry>`. Mirrors the entry class family without changing `LogEntry` in `log.rs`.
- `Boundary { date, process, pid }` and `sequences(&[Entry]) -> Vec<&[Entry]>`, which splits a sequence at its
  boundary entries (the boundaries themselves are dropped).
- `Activity { date, id, parent, name, process, pid }`, `new_activity_id()` (high 32 bits process id, low 32 bits a
  process-local counter, so identifiers from two processes do not collide) and `activity_chain(&[Activity], id)`,
  which walks the parent identifiers to the root and stops at an unknown parent or a cycle.
- `parse_format(&str) -> Result<Vec<Component>, String>` and `render(components, args, show)`: a format string
  becomes `Component { format_substring, placeholder }` (text before each placeholder, as `formatSubstring`). Specs are
  `%d %i` (integer), `%u %x %X %o %p` (unsigned), `%f %e %E %g %G %a` (double), `%s %@` (text), and `%P` (this
  project's byte-sequence spec, shown as hex). Privacy is `%{public}` or `%{private}`, and values are private by
  default. `%%` is a literal percent sign, and length modifiers are accepted and ignored.
- `Argument` (`Int`, `Uint`, `Double`, `Text`, `Data`) with the accessors `int64()`, `uint64()`, `double()`,
  `string()`, mirroring `argumentInt64Value`, `argumentUInt64Value`, `argumentDoubleValue`, `argumentStringValue`.
- `Payload::new(entry, format, args)`: renders the message with private values as `<private>`, and
  `Payload::show_private()` renders it with every value. A count or type mismatch between placeholders and arguments
  is an error.

### `springboard/core/src/log_cursor.rs`

- `Position { micros, ordinal }`: a point in a sequence. Ordered by time, then by the ordinal among entries at the
  same microsecond. `start()` and `end()` bound every sequence. `encode()` and `parse()` give a short string
  (`<micros>.<ordinal>`) that a reader can save and read back.
- `Direction { Forward, Reverse }` (the `OSLogEnumerator.Options` reverse option).
- `Enumerator::new(Vec<Entry>, Direction)`: sorts the entries oldest first and assigns positions. `all()`,
  `entries_after(pos)` (the entries strictly after a saved point, in the enumerator's direction), `positions()`,
  `last_position()` for resuming.

## What is not wired

These are models only. Nothing in `LogStore`, `Logger` or the Console app emits or reads these kinds yet:

- `LogStore::entries` still returns `Vec<LogEntry>` and `Logger` still writes plain messages. The format-string
  payload and the activity and boundary kinds have no producer and no journal encoding.
- `OSLogStore.getEntries(at:)` is not on `LogStore`. The enumerator works on the vector a caller already read, so it
  is not lazy and does not stream from the journal cursor.
- Positions are ordinals within a vector. A position saved from one read stays valid only while entries are appended
  after it, which is the case for the in-process ring and for the journal in time order.
- Per-argument masks (`private(mask:)`) and per-value privacy are not modelled beyond the `public` and `private`
  options.

These gaps are the reason every row below is `partial` and not `parity`.

## Matrix rows closed (framework slug `oslog`)

| Feature (as in `docs/parity-matrix/data/oslog.json`) | Apple | New status | BifrOSt path |
|---|---|---|---|
| Boundary entries (OSLogEntryBoundary) | OSLogEntryBoundary | partial | `springboard/core/src/log_entries.rs` (`Boundary`, `sequences`) |
| Activity entries (OSLogEntryActivity) | OSLogEntryActivity, os_activity | partial | `springboard/core/src/log_entries.rs` (`Activity`, `new_activity_id`, `activity_chain`) |
| Payload entries with format strings | OSLogEntryWithPayload, OSLogMessageComponent | partial | `springboard/core/src/log_entries.rs` (`Payload`, `parse_format`, `render`, `Argument`) |
| Enumerator and persistent positions | OSLogEnumerator, OSLogPosition, OSLogStore.getEntries(at:) | partial | `springboard/core/src/log_cursor.rs` (`Position`, `Enumerator`, `Direction`) |

## Tests

Each module has a `#[cfg(test)] mod tests` with real behaviour checks: format parsing and escaping, refused format
strings, private and public rendering, argument type checks, boundary partitioning, activity chains and cycle
guards, activity id uniqueness, position string round trips, equal-time ordinals, forward and reverse walks, resuming
from a saved position, and mixed entry kinds in one enumerator. The build was not run for this round (the wave
rules forbid builds in this worktree), so the tests are unverified until the lead builds the crate.
