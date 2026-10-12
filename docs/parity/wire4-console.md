# Wire 4: Console app (saved searches, level and message tokens)

Branch `wire4/console`. App `springboard/apps/console-desktop`, Console 1.2.1 becomes 1.3.0. Only that folder changed,
plus this note. Not built: the lead builds once (`cargo test -p bifrost-console` from `springboard/`).

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | BifrOSt path | Status after this wire |
|---|---|---|---|
| `oslog` | Predicate queries | `springboard/core/src/log.rs` (`Query`, `Query::parse_predicate`, `Query::matches`), app model `apps/console-desktop/src/model.rs` (`Query::parse`, `Query::matches`) | Partial: the search now takes `level:` (same as `type:`) and `message:` (the message text only), on top of `process:`, `subsystem:`, `category:` and free text. Still not the predicate language: no `>=`, no `OR`, no `NOT`. |
| `oslog` | Log store (read historical entries) | `springboard/core/src/log.rs` (`LogStore::entries`), app `apps/console-desktop/src/model.rs` (`SavedSearch`, `save_search`, `remove_search`) and `console.rs` | Partial: saved searches are a sidebar section that replays a search over the entries the window has read. No archive is opened (see gaps). |
| `oslog` | Live streaming of entries | `springboard/apps/console-desktop/src/console.rs` (`drain`) | Partial: the search applies to each message as it arrives, so a saved search narrows the live stream the same way the search field does. |
| `os` | Logging: reading logs (OSLogStore, log show and stream) | same as above | Partial: reading, searching and saving a search work in the window. Reading a log archive file is still a gap. |

Pure models reused, not rewritten: the console's own `model::Query` (already the search parser) and `model::Level::parse_word`.
New pure code in the app's model: `SavedSearch`, `save_search` (trims, rejects blank and duplicate text) and `remove_search`.
The core `log::Query::parse_predicate` is not used by the window: `log show` has its own clause grammar, and the window's
search has its own token syntax. Reusing the predicate parser would change what the search field accepts, so it stays out.

## How a user reaches it

- **Search tokens** (Log > Find Message, or the toolbar search field):
  - `level:error` keeps Error messages (`type:` still works and means the same).
  - `message:timeout` keeps messages whose text contains `timeout`. The process, subsystem and category are not checked for
    it. A bare `message:` has no value and is free text, as before.
  - Tokens combine: `level:error message:disk` keeps Error messages whose text contains "disk".
- **Log > Save Search** (enabled when the search field holds text that is not already saved): adds a row to a **Saved
  Searches** section in the sidebar, between Reports and Log Files. The row's text is the search text. The section only
  appears once a search is saved, so the default sidebar is unchanged.
- **Choosing a saved row**: shows **This Mac** (the live journal), puts the saved text in the search field and filters the
  table. The footer shows the count, as for any search.
- **Log > Delete Saved Search** (enabled when the search field holds a saved search's text): removes that row. The section
  disappears when the last one goes.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`). Midget is the
   test machine; the VM is skipped by owner rule.
2. `bifrost pc midget open-app is.olibuijr.console.desktop`, then `bifrost pc midget state` for the window geometry.
3. Tap the toolbar search field (`bifrost pc midget tap-window APP X Y`) and type `level:error`. Only Error rows remain.
   Type ` message:disk` after it: only rows whose text contains "disk" remain.
4. Log menu > Save Search. The sidebar gains a "Saved Searches" header with a `level:error message:disk` row.
5. Select This Mac, clear the search field, then tap the saved row. The field shows the saved text and the table filters
   to the same rows as step 3.
6. Log menu > Delete Saved Search while the field holds the saved text. The row and its header go away.
7. Screenshot each state with `bifrost pc midget screen /tmp/console-wire4.png` and compare the sidebar section with the
   Reports section for header and row style.

## Host tests

Unit tests (run with `cargo test -p bifrost-console` from `springboard/` when the lead builds once):
- `src/model.rs`: `level_and_message_tokens_filter_like_type_and_the_message_predicate` (`level:error` sets the level,
  `message:timeout` matches the text only, a bare `message:` is free text) and
  `saved_searches_keep_trimmed_unique_text_and_can_be_removed` (trimmed text, no duplicate, no blank, removal).
- `src/wire_tests.rs`: `a_saved_search_is_kept_from_the_search_field_and_chosen_from_the_sidebar` (Save is offered once,
  the saved row replays `type:error` over the rows `[1, 2]` out of `[0, 1, 2]`, Delete removes it),
  `a_blank_search_cannot_be_saved`, and `the_message_token_and_the_level_token_narrow_the_table` (`level:error message:disk`
  keeps only row 1).

Note for the lead:
- The Log menu gains two items (Save Search, Delete Saved Search). `tests/compliance.rs` snapshots the window, so the
  window golden should not change; the sidebar is the only place a change would show, and the section is hidden while no
  search is saved.
- `springboard/Cargo.lock` pins `bifrost-console` at 1.2.1. Building without `--locked` updates it to 1.3.0; the lock
  update belongs in the release commit, not this branch.

## REA evidence

- `ev_fc49c032f886234be11ce8afaf32d7f2d6779ad853d6c08d0014b5cf335d0ab7`: OSLogStore page (`documentation/oslog/oslogstore`),
  read with `tools/parity/rea-page.sh`. It lists `getEntries(with:at:matching:)` (an entry sequence filtered by an
  `NSPredicate`), `init(url:)` ("Creates a log store based on a log archive") and `local()`.
- `ev_26d0e381fd8b0dc1a5122578594a1cf725525faf82a7812940cfc0fb426d3385`: OSLogStore `init(url:)` page
  (`documentation/oslog/oslogstore/init(url:)`), read with `tools/parity/rea-page.sh`. It takes a log archive URL and throws.
- The DocC collection page, `python3 -I tools/parity/docc.py oslog/oslogstore`, gives the same list and the entry type
  `OSLogEntry` (`composedMessage`, `date`, `storeCategory`).

## Design notes

- The sidebar section uses the kit's `SidebarItem::header` and `row`, with the `magnifyingglass` glyph. No new colours,
  type sizes or radii.
- Saved rows reuse the search field. Choosing one adds no view, so the table, footer and detail pane are unchanged.
- The Log menu items are enabled from the search field's text, so the menu shows which action is possible.
- The window background stays `CONTENT_BG` (already in `draw.rs`); this wire changes no fill.

## Remaining gaps

- **Log archives (Save and Open)**: no log archive model exists in BifrOSt. `OSLogStore(url:)` reads a log archive, and
  `springboard/sdk/src/archive.rs` is ZIP for folders and files, not log entries, so it is not reused. Opening a plain log
  file already works through the Log Files section, but nothing writes a log archive.
- **Persistence**: saved searches live in the window's memory and are gone when the Console quits. Writing them to disk
  needs a per-user store the app can own; none is wired here.
- **Predicate language**: no `>=` on `level`, no `OR`, no `NOT`, and the tokens are always joined by AND. The core
  `log::Query::parse_predicate` already parses `message CONTAINS` and `level >= error` for `bifrost-log`; the window does
  not use it.
- **Live stream and history**: the search runs in the window after each message is read. The journal read is not filtered
  by the search, and the window reads the newest 500 entries (`LOAD`), so a search does not reach older messages.
- **Persistent positions and lazy enumeration** remain as in `docs/parity/wire-app-console.md`.

## Matrix rows touched

| slug | feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| `oslog` | Predicate queries | `springboard/core/src/log.rs` (`Query`); app `springboard/apps/console-desktop/src/model.rs` (`Query::parse`, `message:`, `level:`) |
| `oslog` | Log store (read historical entries) | `springboard/core/src/log.rs` (`LogStore::entries`); app `springboard/apps/console-desktop/src/console.rs` (Saved Searches) |
| `oslog` | Live streaming of entries | `springboard/apps/console-desktop/src/console.rs` (`drain`, search applied per message) |
| `os` | Logging: reading logs (OSLogStore, log show and stream) | `springboard/apps/console-desktop/src/model.rs`, `console.rs` |
