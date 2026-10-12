# Wire 6: Reminders (1.13.0) wires #tags into the sidebar as a Tags section

Branch `wire6/reminders`, on top of main (release 0.5.43). The change touches `springboard/apps/reminders-desktop/`
and this note. Nothing was built or run (the lead builds once). Host tests were written by hand, not run.

## State of the focus items before this wave

Most of the focus list was already in the app from earlier waves. Checked against the code, not reassumed:

| Focus item | State | Where |
|---|---|---|
| Subtasks, reminders nested under a parent | Already wired: one level, indent and outdent, progress, collapse | `src/model.rs` (`indent`, `outdent`, `progress`, `visible`) |
| Due-date alarms with an early reminder | Already wired in 1.10.0 (notification triggers, Early row) | `src/alerts.rs`, see `docs/parity/wire-app-reminders.md` |
| Completed items show and hide | Already wired: View > Show Completed, Clear Completed | `src/model.rs` (`Options::show_completed`), `src/app.rs` (`Target::OptCompleted`) |
| Smart lists as fetch predicates | Already wired in wave 4 | `src/model.rs` (`Smart::predicate`), see `docs/parity/wire4-reminders.md` |
| Tags | Not wired before this wave | this wave |
| List groups | Not wired (no model, see gaps) | none |

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| eventkit | Reminders | `springboard/apps/reminders-desktop/src/model.rs` (`tag_key`, `tags_in`, `Reminder::tags`, `tag_counts`, `tag_name`, `View::Tag`), `src/chrome.rs` (`TAG_ROW`, Tags section in `lists_source`, `sidebar_target`, `controls`), `src/app.rs` (`Target::Tag`), `src/view.rs` (list title) |

The matrix has no tags row. The closest row is EventKit "Reminders" (status `os`, app-internal store). Its status is not
changed here, because the matrix is not edited in this wave. The lead can decide whether a row for Reminders tags is
added. EventKit itself does not expose tags: `EKReminder` (docs read: `eventkit/ekreminder`) has priority, dates,
completion and no tag property. Tags are an app feature of this app, not an SDK API, so no SDK surface was added.

### Tags

- A tag is a `#word` in a reminder's title or notes. The `#` starts the word (a space or punctuation before it, not a
  letter), then a letter, then letters, digits, `_` or `-`. A trailing `-` is not part of the tag. `C#` and `##a` are
  not tags, and `#2024` is not a tag because it does not start with a letter.
- Tags match without case: `#Home` and `#home` are one tag. The first spelling is shown.
- Tags are derived from the text on each rebuild, so the store file format does not change. Nothing new is saved.
- The sidebar has a Tags section under My Lists, shown only when some reminder carries a tag (so the default sidebar and
  its accessibility tree are unchanged). Each tag row shows the number of open reminders with it.
- Choosing a tag opens `View::Tag(key)`: every reminder with that tag, in the list's rows. Completed reminders follow
  View > Show Completed, as in a list. The list title is the tag, such as "#home", in the blue tint.
- A new reminder made while a tag is open goes to the default list, as in a smart list.
- The tag key is an FNV-1a hash of the lowercase tag. `View` stays `Copy`, and a key does not move when tags are
  added or removed. The sidebar row ids are `TAG_ROW + index` in the sorted tag list (`tag_counts`).

## How a user reaches it

1. Open Reminders. Press New, type `Buy milk #home` as the title, then Save.
2. The sidebar shows a Tags section under My Lists with "#home" and a count of 1.
3. Choose "#home": the list shows the reminders with that tag. The title reads "#home".
4. View > Show Completed shows the completed ones with the tag as well.
5. Add `#home` to another reminder's notes: the count goes up at once.

## How to verify on the test PC (Midget)

The owner rule is to test on Midget only (no VM).

1. Publish and update as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Check the
   version with `bifrost pc midget state` (expect Reminders 1.13.0).
2. `bifrost pc midget open-app is.olibuijr.reminders.desktop`. Add "Milk #home" and "Bread #Home" with New, Save.
   The sidebar's Tags section shows "#home" with count 2.
3. Choose "#home" with `bifrost pc midget tap-window APP X Y` (window-local pixels). The list shows both reminders
   and the title reads "#home".
4. Complete "Milk": the count reads 1. View > Show Completed shows it again with the tag.
5. Capture with `bifrost pc midget screen /tmp/reminders-tags.png`. Check Light and Dark: the content stays `CONTENT_BG`.

Host tests (not run): `cargo test -p bifrost-reminders`. The new tests are in `src/model.rs`:
- `tags_are_hashtags_in_title_and_notes`: `tags_in("Buy #Milk, #milk and #Bread-1 #2024 C#sharp # x")` is
  `["Milk", "Bread-1"]`; `tags_in("##a")` is empty; `tags_in("#work-")` is `["work"]`; a reminder titled "Call #mom"
  with notes "#mom and #Family" has tags `["mom", "Family"]`; `tag_key` ignores case.
- `tag_counts_count_open_reminders_and_sort_by_name`: "#Home chores" (open), "#home #work" (completed), "Work #Work"
  (open). The counts are `[("Home", 1), ("work", 1)]`; `tag_name` for WORK is "work", for gym is None.
- `tag_view_lists_the_reminders_with_that_tag`: with the default options only reminder 1 shows; with Show Completed,
  reminders 1 and 2 show, in storage order.

The existing `model::tests` and `actions_tests` do not change: the default fixture has no `#` in any title, so the
Tags section is absent and their row counts are unchanged. The compliance test counts controls, and it sees no new
controls for a fixture without tags. The snapshots under `tests/snapshots/` are for the default sidebar and need no
re-bless for this change (the lead should confirm on the build).

## REA evidence (official documentation)

No REA evidence id was recorded in this wave: the rendered-page tool (`tools/parity/rea-page.sh`) was not run, and
the akurai index search timed out before the code search. The official reads that were run, with the DocC tool
(`python3 -I tools/parity/docc.py`):

- `eventkit/ekreminder`: EKReminder, "a class that represents a reminder in a calendar"; properties priority,
  start and due date components, completion. No tags property. https://developer.apple.com/documentation/eventkit/ekreminder
- `eventkit/ekalarm`: EKAlarm, `init(absoluteDate:)` and `init(relativeOffset:)`, the relative offset from the start.
  This is the early-alert model the app's Early row already follows (alerts.rs, wire-app-reminders.md).
  https://developer.apple.com/documentation/eventkit/ekalarm

## Remaining gaps

- List groups (a group of lists in the sidebar) are not in any model. There is no group record in `Data`, and no
  model under `springboard/ui/src` for it. A later wave needs a model before the sidebar can show groups.
- Tags have no reference API in EventKit or the SDK, so the tag rules here follow the app's own text rules, not an
  official definition. The Tags section is not sorted by usage, and there is no tag rename or tag delete.
- The sidebar has no scroll for the Tags section. With many tags the rows below the lists area are clipped
  (`lists_bounds` ends above Add List). The list is shown and hit-tested, but a long tag list needs a scroll or a
  collapsible section (`SidebarItem::section`) in a later wave.
- The tag view does not nest subtasks: a subtask shows only when its own text has the tag.
- `springboard/Cargo.lock` still records `bifrost-reminders` 1.12.0 (it is outside this folder, so it is not
  touched here). A normal build updates it. A `--locked` build fails until the lock is refreshed.
- `springboard/apps/reminders-desktop/src/tests.rs` is still not compiled (only `actions_tests.rs` is), as before.

## Matrix rows

| Slug | Feature (exactly as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| eventkit | Reminders | `springboard/apps/reminders-desktop/src/model.rs` (`tags_in`, `tag_counts`, `View::Tag`); `src/chrome.rs` (Tags section, `sidebar_target`) |
