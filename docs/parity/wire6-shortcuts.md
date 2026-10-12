# Wire 6: Shortcuts app (intent result views and follow-on results)

Shortcuts 1.6.0 (`springboard/apps/shortcuts-desktop/`) shows the result of "Plan Shortcut" as a result panel in the
window, with one numbered row per step, and a follow-on "Run Shortcut" button that runs the same shortcut next. The
model is the existing `IntentResult` with `snippet` (`IntentResult.Snippet`) and `opens` (`result(opensIntent:)`).
No new model was written. The change is in the app: `src/automation.rs` (the plan result), `src/widget.rs` (the panel
and its two buttons) and `src/main.rs` (the request path now carries intent parameters). Std only, no unsafe, no new
crate, no DETAIL_BG (the window fills with CONTENT_BG). Not built here: the lead's build runs
`cargo test -p bifrost-shortcuts`.

Matrix rows are not edited here; `docs/parity-matrix/` is updated centrally.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | Model used | BifrOSt path |
|---|---|---|---|
| app-intents | Snippets and result views | `intents::Snippet` (`Snippet::new`, `row`) in `IntentResult::with_snippet` | `src/automation.rs` `perform` (one row per step); `src/main.rs` `run_intent`; `src/widget.rs` `show_snippet`, `draw_result` |
| app-intents | Follow-on and opens-app results | `IntentResult::opens` (`result(opensIntent:)`), checked by `Intent::check_result` | `src/automation.rs` `perform` (opens `run-shortcut` with the shortcut id) and `title_of`; `src/widget.rs` `Target::ResultRun`, `take_intent_requests` |
| siri-kit | Parameter resolution, confirmation and disambiguation | `bifrost_app::controls::intent_resolve` (unchanged this round; see wire 3 and 5) | `src/automation.rs` `resolve` (unchanged) |

### Behaviour

- **Plan result panel.** Tapping "Plan Shortcut" in the Intents section of the action library opens a panel at the bottom
  of the window instead of the one-line toast. It shows the shortcut's name (title), the dialog ("Bedtime runs 3 steps."),
  and one row per step: the number, then the step text (for example "Set Wi-Fi: Off"). More than eight steps show the
  first eight and "and N more". A plan with no actions still says "has no actions" in the toast.
- **Follow-on button.** The panel's left button is the follow-on, titled from the intent it opens ("Run Shortcut"). Tapping it
  runs `run-shortcut` with `{"shortcut": "<id>"}`, the same shortcut. Run cannot run on this device yet, so its reason
  shows in the toast. The right button, Done, closes the panel.
- **Lifetime.** The panel closes on Done, on the follow-on tap, or after eight seconds. A later toast (for example a run
  reason) replaces it. Taps outside the panel's two buttons do not reach the grid underneath while the panel is open.
- **Control socket.** `intent.plan-shortcut` now also returns a `snippet` and an `opens` key next to `value` and `dialog`.
  The value and dialog text are unchanged.
- **Requests carry parameters.** A window request is `(intent id, parameters)`, so a follow-on can name the shortcut it
  runs. The action library rows still send empty parameters, which use the shortcut in front, as before.

## How a user reaches it

1. Open Shortcuts, open a shortcut's editor (Edit, then tap its tile).
2. Tap "Add Action", then tap "Plan Shortcut" under Intents.
3. The panel shows the steps. Tap "Run Shortcut" to run the same shortcut (it reports that this device cannot run shortcut
   actions yet), or tap Done.

## How to verify on the test PC (Midget)

Midget is the only test machine; no VM.

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. `bifrost pc midget open-app is.olibuijr.shortcuts.desktop`, then tap the Bedtime tile after Edit with
   `bifrost pc midget tap-window Shortcuts X Y`. Tap "Add Action", then "Plan Shortcut", and take
   `bifrost pc midget screen /tmp/shortcuts-1.6.0-plan.png`. Expect the title "Bedtime", the dialog "Bedtime runs 3 steps.",
   and three numbered rows (Set Do Not Disturb: On; Set Brightness: 10%; Set Wi-Fi: Off), with "Run Shortcut" and "Done" at the bottom.
3. `tap-window` on "Run Shortcut" in the panel: the toast reads "Bedtime cannot run on this device yet." and the panel closes.
   Repeat step 2 and tap "Done": the panel closes and the picker stays.
4. Let a panel sit for eight seconds: it closes by itself.
5. Control socket check: `bifrost pc control call` with `intent.plan-shortcut` and `{"shortcut":"Bedtime"}` returns
   `value`, `dialog`, and a `snippet` with three rows and an `opens` of `run-shortcut` with `{"shortcut":"0"}`.

Host tests (for the build stage): `cargo test -p bifrost-shortcuts`. New in `automation::tests`:
`plan_result_has_a_snippet_row_per_step_and_a_run_follow_on` (the snippet rows and the follow-on, from the default Bedtime
shortcut, which is id "0") and `the_follow_on_names_a_declared_intent_title`. The existing plan, run, and catalog tests
still hold: the plan value and dialog are unchanged.

## Official reference

Checked with `python3 -I tools/parity/docc.py appintents/intentresult` (DocC facts, no REA capture this round): IntentResult
has `result(dialog:view:)` for a result view next to the dialog, and `result(opensIntent:)` and
`result(opensIntent:dialog:)` for a follow-on intent. The app uses the follow-on and the snippet shape; the window draws
the rows itself (the shell draws snippets, the app builds the model).

## REA evidence

No new REA capture was made in this round. Earlier ids are reused from `docs/parity/wire4-shortcuts.md` and
`docs/parity/wire-app-shortcuts.md`:

- `ev_e513da61d1dff052820cb3e04e7eda85eb389fb2733589aaa89f3a145d9d2e11`: rendered official page
  https://developer.apple.com/documentation/appintents/appintent (perform() returns a result).
- `ev_410046ef507f693f7917b8b944b3ccda05f53c5f35f94875ca3b9e5204b93c1e`: rendered page
  https://developer.apple.com/documentation/appintents/intentparameter.

## Remaining gaps

- **Confirmation before a destructive intent.** No intent in this app is destructive, so no confirmation step is shown.
  The editor's "Delete Shortcut" button is destructive and still deletes without a confirmation (it is not an intent).
  Run Shortcut is not confirmed either, because it cannot run yet.
- **Disambiguation in the window.** The window's intent rows send no `shortcut` parameter, so a name shared by several
  shortcuts is never asked about in the window. The control socket still returns the choose-an-id error (wire 3).
- **Entity picker in the window.** There is no picker for the Shortcut parameter in the window; entity queries are served
  over the control socket only.
- **Authentication and modes.** Run and Plan are background intents with no authentication policy. The window does not check
  a session before running an intent.
- **Open App follow-on.** The follow-on opens an intent of this same app. An "open another app" follow-on is not built.
- **Snippet rows are fitted, not wrapped.** Step text longer than the panel is cut with an ellipsis.
- **Not built or run in this stage.** `Cargo.lock` is not updated; the lead's build updates it for the 1.6.0 bump. The panel
  geometry (`result_box`, `result_buttons`) has no unit test; it is checked on Midget (step 2 to 4 above).

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| app-intents | Snippets and result views | `springboard/apps/shortcuts-desktop/src/automation.rs` `perform`; `src/main.rs` `run_intent`; `src/widget.rs` `show_snippet`, `draw_result` |
| app-intents | Follow-on and opens-app results | `src/automation.rs` `perform` (opens `run-shortcut`), `title_of`; `src/widget.rs` `Target::ResultRun`, `take_intent_requests` |
| siri-kit | Parameter resolution, confirmation and disambiguation | `src/automation.rs` `resolve` (unchanged this round) |
