# Wire 4: Reminders (1.12.0) wires Edit Undo and Redo and smart lists as fetch predicates

Branch `wire4/reminders`, on top of main (release 0.5.40). The change touches `springboard/apps/reminders-desktop/`
and this note. Nothing was built or run (the lead builds once). Host tests were written by hand, not run.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| core-data | Undo and redo of model changes | `springboard/apps/reminders-desktop/src/app.rs` (`commit_named`, `step_history`, `edit_commands`, `edit_titles`, `snapshot`, `completion_name`), `src/main.rs` (`commands`, `undo_titles`, `Input::Command`) |
| swiftdata | Undo and redo of data changes | the same code as the row above |
| core-data | Fetch requests and predicates | `springboard/apps/reminders-desktop/src/model.rs` (`impl Record for Reminder`, `Smart::predicate`, `Smart::count` through `fetch_indices`) |
| swiftdata | Predicate and descriptor queries, sorting, sections (Query, FetchDescriptor) | `src/model.rs` (`Smart::predicate`, `Smart::matches`); the smart lists only, not a general query engine |

Both undo rows stay `partial`: the undo history is an app-level stack of snapshots and inverse closures, not a
change tracker over an object graph. Nothing records store changes automatically; each edit path names its change.
The fetch row stays `partial`: a smart list is a predicate over in-memory reminders. There is no fetch request for
user lists, no offset or batch fetching, and no sectioned results.

### Undo and Redo

- Edit > Undo and Edit > Redo are the shell's items (`Command::Undo`, `Command::Redo`, Cmd+Z and Cmd+Shift+Z). The
  app answers `commands()` (enabled from the history) and `undo_titles()` ("Undo Mark as Completed", "Redo
  New Reminder"), and handles `Input::Command` by calling `step_history`. The app adds no Edit menu of its own.
- Every change that goes through `commit_named` is one undo step with a name and a before and after snapshot:
  - Mark as Completed / Mark as Incomplete (the circle, Space and the swipe-right completion).
  - New Reminder and Edit Reminder (the editor's Save, including the flag, the priority and the date).
  - Duplicate Reminder, Move Reminder (drag to reorder), New List and Edit List.
- Delete, Clear Completed and the swipe-flag keep their own steps, as before (`restore_items`, `flag_toggle`). They
  are on the same history, so Undo and Redo move through all of them in order.
- A new change clears the redo steps (`UndoManager` behaviour). A change that registers replaces the Undo bar, which
  always offers the latest change.
- Undo and Redo are enabled only on the list. While a reminder's details or a list editor is open, they are off, so
  a stale draft is never overwritten.
- The Undo bar's button runs the same `step_history(false)`.

### Smart lists as predicates

- `Smart::predicate(today)` builds a `fetch::Predicate` for each smart list:
  - Today: open, and `due <= today`.
  - Scheduled: open, and it has a day (`scheduled` is true).
  - Flagged: open and flagged.
  - All: open.
  - Completed: completed.
- A reminder without a day has no `due` field, so a day test never matches it (the `fetch` rule for a missing field).
- `Smart::matches` and `Smart::count` use the predicate. `count` goes through `fetch::fetch_indices`. The sidebar
  counts and the list are recomputed from the data on every change (`commit` bumps the generation and `rebuild`
  refreshes the rows), so they are live. The Today list also follows the day: `today` is passed in on each rebuild.

## How a user reaches it

1. Open Reminders. Tap a reminder's circle to complete it, then press Edit > Undo (Cmd+Z). The reminder is open
   again, and the Edit menu reads "Redo Mark as Completed". Press Redo (Cmd+Shift+Z) to complete it again.
2. Press New, type a title, press Save. Edit > Undo reads "Undo New Reminder" and removes it.
3. Open the Today, Scheduled or Flagged tile. Complete one reminder: its count goes down at once. Undo: it comes back.
4. Open a reminder's details. Edit > Undo is off while the details are open (the Undo menu item is disabled).

## How to verify on the test PC (Midget)

The owner rule is to test on Midget only (no VM).

1. Publish and update as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Check the
   version with `bifrost pc midget state` (expect Reminders 1.12.0).
2. `bifrost pc midget open-app is.olibuijr.reminders.desktop`. Add "Milk" with New, Save. Open the Edit menu from the
   bar and check "Undo New Reminder". Choose it: "Milk" goes; choose Edit > Redo: it comes back.
3. Complete "Milk" with its circle. Edit > Undo reads "Undo Mark as Completed". Choose it: the circle is open again.
4. Open Today (the sidebar tile count). Complete a reminder due today; the count drops. Undo: it rises again.
5. Open a reminder's details and check that Edit > Undo is greyed out.
6. Capture with `bifrost pc midget screen /tmp/reminders-undo.png`. Light and Dark: the content stays `CONTENT_BG`.

Host tests (not run): `cargo test -p bifrost-reminders`.
- `model::tests::smart_lists_are_fetch_predicates_over_the_reminders`: with today = 20 000 and six reminders (overdue
  19 999, today 20 000, later 20 001, flagged without a day, plain without a day, completed due 19 990), the counts are
  Today 2, Scheduled 3, Flagged 1, All 5, Completed 1. Today at 20 001 gives 3 and at 19 999 gives 1.
- `actions_tests::edit_undo_and_redo_step_through_saves_and_completions`: New Reminder, then Mark as Completed; undo
  and redo move through both; undoing both empties the list; redo brings "Milk" back; no step while the editor is open.
- `actions_tests::a_new_change_after_an_undo_drops_the_redo_steps`.

## REA evidence (official documentation, rendered)

- `ev_a17c0b34a7418c8906ed6492410b2df2ca95db8c8397d3fa517d2dbdfcd615ae`: NSFetchedResultsController (a controller
  that manages the results of a fetch request and displays them; the delegate is notified when the results change;
  the cache; `performFetch`). https://developer.apple.com/documentation/coredata/nsfetchedresultscontroller
- DocC read (`python3 -I tools/parity/docc.py foundation/nspredicate`): NSPredicate, "a definition of logical conditions
  for constraining a search for a fetch or for in-memory filtering". Its `init(format:...)` family is the text form; the
  app uses the typed `fetch::Predicate` instead.
- The undo page cited by the matrix (`coredata/undo-and-redo-of-individual-and-batched-changes`) returned 404 from
  DocC and from the rendered page, and `appkit/nsundomanager` also 404 in DocC. No undo evidence id is recorded; the
  undo behaviour is taken from the in-repo model (`springboard/ui/src/undo.rs`) and the Edit menu contract in the
  shell (`springboard/sdk/src/view/app.rs`). A later wave should find the current undo page and record it.

## Remaining gaps

- Undo of model changes is a snapshot per edit, not a change tracker. Store changes made outside these paths (the
  intent `add-reminder`, the Today widget) are not on the history. The intent is not a user edit, so it is left off.
- The Undo bar still offers only the latest change for its usual time. The redo of a deleted reminder is not shown
  in the bar; it is in Edit > Redo.
- Fetched results controller for lists: the app's `model::visible` sorts and groups the rows itself. There is no
  results controller with a delegate, no section cache and no insert, move and remove notifications. The table view
  is rebuilt on each change. The row stays `partial`.
- Live results observation: the sidebar counts and the list are recomputed from the data, which is live in the app.
  There is no observer object over a fetched collection, so the row stays `partial` and is not wired as an observer.
- Predicate text (the `NSPredicate` format string) is not parsed. Smart lists are fixed predicates in code.
- Asynchronous fetch: the lists are read from memory; there is nothing to fetch in the background.
- `springboard/apps/reminders-desktop/src/tests.rs` is still not compiled (only `actions_tests.rs` is), as before.

## Matrix rows

| Slug | Feature (exactly as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| core-data | Undo and redo of model changes | `springboard/apps/reminders-desktop/src/app.rs` (`commit_named`, `step_history`, `edit_commands`); `src/main.rs` (`Input::Command`, `commands`, `undo_titles`) |
| swiftdata | Undo and redo of data changes | `springboard/apps/reminders-desktop/src/app.rs` (`commit_named`, `step_history`) |
| core-data | Fetch requests and predicates | `springboard/apps/reminders-desktop/src/model.rs` (`Record for Reminder`, `Smart::predicate`, `Smart::count`) |
| swiftdata | Predicate and descriptor queries, sorting, sections (Query, FetchDescriptor) | `springboard/apps/reminders-desktop/src/model.rs` (`Smart::predicate`, `Smart::matches`) |
