# Wire 4: Shortcuts app (App Intents catalog and running an intent from the window)

Shortcuts 1.5.0 (`springboard/apps/shortcuts-desktop/`) shows the app's intent catalog in the action library and runs
an intent from there. The run goes through the same `automation::perform` path the control socket and the shell use
(`intents::dispatch` reaches it through `perform_intent`), and the result dialog is shown in the window. No new model
was written; the change is two small adapters in the app plus the window's Intents section. Std only, no unsafe, no
new crate. Not built here: the lead's build runs `cargo test -p bifrost-shortcuts`.

Snapshot and compliance: only the grid is snapshotted (`tests/compliance.rs` renders the default view), so the picker
change does not touch an existing snapshot row. Nothing was blessed. The window fills with `CONTENT_BG` already; no
`DETAIL_BG` is used.

## What is now wired

| Model / API | Where | What the app does with it |
|---|---|---|
| `intents::Intent` (`id`, `title`, `description`) | `src/automation.rs` `declared`; `src/widget.rs` picker layout | The Add Action list has an Intents section: one row per declared intent (its title) and a footnote line under it (its description). |
| `intents::dispatch` / `perform_intent` path | `src/automation.rs` `perform`; `src/main.rs` `run_intent` | Tapping an intent row runs it on the shortcut being edited (the shortcut in front, no parameters). |
| `IntentResult::dialog` | `src/main.rs` `run_intent`; `src/widget.rs` `show_result` | The dialog (or the error message) shows in the window's toast. |

### Behaviour

- **Catalog.** The Intents section lists `Run Shortcut` and `Plan Shortcut`. Each row shows the title; the note under it
  shows the description (fitted to one line with an ellipsis if it is too long for the window).
- **Plan from the window.** With an editor open, Add Action, then tap "Plan Shortcut": the toast shows
  "Bedtime runs 3 steps." (the dialog) and the window stays on the picker. With no shortcut in front the picker cannot
  be open, so the "Shortcut is required" case cannot happen from the window.
- **Run from the window.** Tapping "Run Shortcut" shows "Bedtime cannot run on this device yet", the same error the
  control socket gives. The window never reports a run that did not happen.
- **Unchanged.** The built-in action kinds, the grid, the editor, the scripting model and the control socket behave as
  in 1.4.1. Intents do not change the list.

## How a user reaches it

1. Open Shortcuts, open a shortcut's editor (Edit, then tap its tile).
2. Tap "Add Action". The picker lists the built-in actions, then an "Intents" section.
3. Tap an intent. Its result appears at the bottom of the window for about three seconds.

## How to verify on the test PC (Midget)

Midget is the only test machine; no VM.

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. `bifrost pc midget open-app is.olibuijr.shortcuts.desktop`, then `bifrost pc midget tap-window Shortcuts X Y` on a
   tile after Edit to open its editor; `bifrost pc midget screen /tmp/shortcuts-1.5.0-editor.png`.
3. `bifrost pc midget tap-window Shortcuts X Y` on "Add Action", then
   `bifrost pc midget screen /tmp/shortcuts-1.5.0-picker.png`: the picker shows the Built-in Actions, then the Intents
   header with "Run Shortcut" and "Plan Shortcut", each with its description underneath.
4. `bifrost pc midget tap-window Shortcuts X Y` on "Plan Shortcut", then screen: the toast reads "<name> runs N steps.".
   Tap "Run Shortcut": the toast reads "<name> cannot run on this device yet."
5. Control socket check (unchanged): `bifrost pc control call` with `intent.plan-shortcut` and `{"shortcut":"Bedtime"}`
   returns the same dialog.

Host tests (for the build stage): `cargo test -p bifrost-shortcuts`. New tests in `automation::tests`:
`the_catalog_gives_each_intent_its_title_and_description` and
`an_intent_from_the_action_library_runs_on_the_shortcut_in_front`. Hand check: `model::defaults()` has "Bedtime" with
three actions (the existing `plan_lists_the_steps_in_order_without_running_them` asserts this), so the plan dialog is
"Bedtime runs 3 steps." and the run error is "Bedtime cannot run on this device yet".

## REA evidence

- `ev_e513da61d1dff052820cb3e04e7eda85eb389fb2733589aaa89f3a145d9d2e11`: rendered official page
  https://developer.apple.com/documentation/appintents/appintent (title and description, `perform()` returning a result,
  parameters, parameter summary; captured for this change).
- `ev_83d11d67cabda1fd8a41f978481915a2be92efa2f1adab8fc92ed741cacae191`: AppEntity page (reused from `wire2-shortcuts.md`).

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| app-intents | Intent declaration metadata (title, description) | `springboard/apps/shortcuts-desktop/src/automation.rs` `declared`; shown in `src/widget.rs` picker (Intents section) |
| app-intents | Perform and IntentResult (value and dialog) | `src/automation.rs` `perform`; `src/main.rs` `run_intent`; dialog in `src/widget.rs` `show_result` |
| app-intents-testing | Intent test helpers (typed result from a run) | `src/automation.rs` tests `an_intent_from_the_action_library_runs_on_the_shortcut_in_front` |

## Remaining gaps

- Entity pickers with suggested and by-string search are not in the window. Entity queries (`suggested`, `by id`,
  `by string`) are served only over the control socket (`automation::entities`). A window picker for the Shortcut
  parameter would need a search field in the picker; not built in this round.
- The intent run from the window does not show the plan steps, only the dialog. The steps are in the `value` of the
  result and are not displayed.
- Descriptions longer than the window are truncated with an ellipsis rather than wrapped.
- Running an intent in the foreground or in the background is not distinguished; both paths run the same dry-run plan.
- The window run still does not run any action (no action runner on the shell).
- Not built or run in this stage. `Cargo.lock` is not updated; the lead's build updates it for the 1.5.0 bump.
