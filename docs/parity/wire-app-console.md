# Wire: Console app (log positions, walk, subsystem filter, signposts)

Branch `wire/app-console`. App `springboard/apps/console-desktop` (Console 1.1.0). Only that folder changed, plus this note.

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | BifrOSt path | Status after this wire |
|---|---|---|---|
| `oslog` | Enumerator and persistent positions | `springboard/core/src/log_cursor.rs` (`Position`, `Enumerator`), adapter `apps/console-desktop/src/cursor.rs` | Partial: positions are computed in the window for the loaded messages. No persisted position tokens across launches, no lazy enumerator. |
| `oslog` | Signpost entries and OSSignposter | `springboard/core/src/signpost.rs` (`Signposter`, `Summary`, `Interval`), SDK `bifrost_app::signpost`, reader `apps/console-desktop/src/signposts.rs` | Partial: the Console times its own view loads as the interval `view.load`, and the Signposts view reads interval and event messages from the journal. |
| `os` | Signposts (OSSignposter, os_signpost) | same as above | Partial: intervals with durations, totals per name. Begin and end in other apps are not wired (they use the SDK on their own). |
| `oslog` | Log store (read historical entries) | `core/src/log_entries.rs` (`Entry::Log`) through `cursor.rs` | Partial: the walk and "after position" work on the entries the journal read returns. |

Pure models reused, not rewritten: `log_cursor::{Position, Enumerator, Direction}`, `log_entries::Entry`, `signpost::{Signposter, Summary, Interval, Id}`.

Not wired (no source data): activities (`log_entries::Activity`, `activity_chain`). Journal lines carry no activity identifier, so there is nothing to filter by. Stays a gap.

## How a user reaches it

- **Log menu > Show Messages After Selected**: select a message, then this shows only the messages newer than it (by its position). The item becomes **Show All Messages** to return. Works while the journal streams.
- **Log menu > Show Only This Subsystem**: appends `subsystem:<id>` to the search field for the selected message's subsystem (disabled when the message has no subsystem).
- **Detail pane > Position**: the selected message's position in the log's order, as `<micros>.<ordinal>`.
- **Detail pane > Duration**: for a signpost interval message.
- **Sidebar > Reports > Signposts** (clock glyph): the signpost intervals and events among the newest 500 journal entries. The footer shows per-name totals: count, total, longest.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`). Midget is the test machine; the VM is skipped by owner rule.
2. `bifrost pc midget open-app is.olibuijr.console.desktop`, then `bifrost pc midget screen /tmp/console.png`.
3. Live: select a row; Log > Show Messages After Selected. The table shows only newer rows. Log > Show All Messages restores them.
4. Detail pane shows a Position line for the selected row.
5. Log > Show Only This Subsystem on a row from another app: the search field reads `subsystem:...`, and only that subsystem's rows remain.
6. Signposts row under Reports: the footer reads `view.load <n>x, ... total, longest ...` once the Console has run a load (the Console logs its own `view.load` on every view open). Select such a row: the pane has a Duration line.
7. Tap-window for every control with `bifrost pc midget tap-window`; the menu items are reachable through the Log menu.

## Host tests

Unit tests in the app (run with `cargo test -p bifrost-console` from `springboard/` when the lead builds once):
- `src/cursor.rs`: conversion to the core entry, ordinal per microsecond, walk after a position, saved position text, empty buffer.
- `src/signposts.rs`: parsing interval and event messages, malformed messages, duration units, summary totals, the Console's own timed load.
- `src/wire_tests.rs`: rebuild with the walk, the Signposts view and footer, the subsystem token.

Note for the lead: `tests/compliance.rs` snapshots the window. The new sidebar row and the new menu items change the golden; re-bless with `BIFROST_BLESS=1` after the first build, and check the snapshot diff is only the new row and the new items.

## REA evidence

- `ev_374b2d3c607bd6f150a26a9965e14452eb700367c587190d05071f67d83f5332`: OSSignposter page (`os/ossignposter`): interval begin and end, the closure form, events, `makeSignpostID`.
- `ev_6ef12e8f5ea088fc5aaf1304d8098464e8e1c86e4f419283e27a04a74708ee9e`: OSLogStore page (`oslog/oslogstore`): `position(date:)`, `getEntries(with:at:matching:)`.
- `ev_8ac54498a8670eb15246f64a32f714b0a5995546c245b4b8e8f237915b4b7beb`: OSLogEnumerator page (`oslog/oslogenumerator`): the reverse option.

Official DocC JSON read with `tools/parity/docc.py` for `os/ossignposter`, `oslog/oslogstore`, `oslog/oslogenumerator`, `oslog/oslogposition`, `oslog/oslogentrysignpost`, `oslog/oslogentrylog`.

## Design notes

- The window uses the kit's existing toolbar, sidebar, table and detail pane. The new sidebar row uses the `clock` glyph from `glyphs::NAMES`. No new colours, no new type sizes.
- Positions are computed on demand (selection, menu actions, rebuild in walk mode, the Signposts view). Nothing is computed per frame.
- Ordering: the walk sorts by date with a stable sort, the same sort `Enumerator::new` makes, so each position maps back to its sequence number.
- The Console logs its own signposts under `is.olibuijr.console.desktop` (declared in the manifest), so they appear in the unified log.

## Remaining gaps

- Activities: no identifier in journal lines, so no activity filter.
- Persistent positions across launches (saving a `Position` token) and `getEntries(at:)` against a log archive.
- A lazy enumerator: the window still loads a Vec of at most 500 (live) entries and 50,000 in memory.
- Signposts in other apps: only the SDK signposter writes them; apps must adopt `bifrost_app::signpost` to show intervals here.
- Signposts view reads the newest 500 journal entries; older intervals are not reached.
