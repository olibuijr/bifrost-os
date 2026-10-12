# Wire 2: Shortcuts app (value queries, onscreen context, actor and async publisher)

Shortcuts 1.4.0 (`springboard/apps/shortcuts-desktop/`) wires four models that were host-tested only: the value
query result (`bifrost_ui::value_query`), the onscreen entity context (`bifrost_ui::onscreen_entity`), the actor
(`bifrost_app::actor`) and the async publisher (`bifrost_app::async_publisher`). Three are used. The async publisher
is not wired (see gaps). Nothing was added as a new model; the app adds small adapters in `src/automation.rs`. Std
only, no unsafe, no new crate. Not built or run here: the lead's build runs `cargo test -p bifrost-shortcuts`.

Name note: the focus text says `ResolvedValueQueryResult`. The model in `springboard/ui/src/value_query.rs` is
`ValueQueryResult` (with `ValueItem` and `QueryError`); the official name is recorded in its doc comment. No second
type was written.

Snapshots and compliance: nothing that draws changed (no window rows, colours or layout), so `tests/snapshots` and
`tests/compliance.rs` keep their existing rows. No snapshot row was added or blessed.

## What is now wired

| Model | Where | What the app does with it |
|---|---|---|
| `value_query::ValueQueryResult`, `ValueItem` | `src/automation.rs` `picker` | The shortcut list becomes a result: id = list index, title = name, properties `color` (palette index) and `actions` (step count). `entity.query` (`entities`) builds its entities from that result. |
| `onscreen_entity::OnscreenContext`, `EntityRef`, `ViewAnnotation` | `src/automation.rs` `onscreen_context`, `frontmost`; `src/widget.rs` `on_screen`; `src/main.rs` `frontmost` | The window annotates what it shows: each grid tile is a visible view, and an open editor or picker annotates its one shortcut as selected. `frontmost` reads the selected Shortcut annotation. |
| `actor::Actor` | `src/automation.rs` `Planner`; `src/main.rs` field `planner` | Dry-run plans (`plan-shortcut`) run on the planner's own thread. The control-socket caller waits for the answer; the window thread does not. |

### Behaviour

- **Parameter picker result.** Ids are list indexes, so each shortcut has one id even when names repeat. Name
  matching is still decided by the existing resolution (`intent_resolve`).
- **Onscreen first.** `run-shortcut` and `plan-shortcut` now have an optional `shortcut` parameter
  (`shortcut:entity(Shortcut)?` in `manifest.toml`). When it is missing and an editor or picker is open, the shortcut
  in front is used. When it is missing and nothing is in front, the reply is "Shortcut is required". When the
  parameter is given, it always wins over the one in front.
- **Grid.** With the grid showing, every tile is annotated as shown and none is selected, so a parameter-less call
  still asks for a shortcut.
- **Planner.** `Planner::plan` returns the same text as `plan_text` on the calling thread (a unit test checks this),
  and a shortcut with no steps still says "has no actions".
- **Run.** Unchanged: `run-shortcut` resolves and then says it cannot run on this device yet.

## How a user reaches it

- In the Shortcuts window: open a shortcut's editor. Other apps and automation that call `intent.plan-shortcut` or
  `intent.run-shortcut` without a shortcut get the shortcut in the editor.
- Through the control socket (see the `bifrost-app-control` skill): `entity.query` with `type: "Shortcut"` and
  `intent.plan-shortcut` with and without `shortcut`.

## How to verify on the test PC (Midget)

Midget is the only test machine; no VM.

1. Publish and update as usual (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`).
2. `bifrost pc midget open-app is.olibuijr.shortcuts.desktop`, then `bifrost pc midget screen /tmp/shortcuts-1.4.0-grid.png`.
   The grid looks as in 1.3.0 (sidebar, toolbar, tiles).
3. Open a shortcut's editor with `bifrost pc midget tap-window Shortcuts X Y` (window-local pixels of its tile after
   Edit), then `bifrost pc midget screen /tmp/shortcuts-1.4.0-editor.png`. The editor looks as before.
4. With the editor open, call `intent.plan-shortcut` with `{}` through `bifrost pc control call`. It returns the steps
   of the shortcut in the editor. Go back to the grid and call it again with `{}`: the reply says "Shortcut is required".
5. `entity.query` with `{"type": "Shortcut", "string": "bed"}` returns the Bedtime entity with "3 actions" as its
   subtitle, as in 1.3.0.
6. `intent.run-shortcut` with `{"shortcut": "Bedtime"}` still returns the "cannot run on this device yet" error.

Host tests (for the build stage): `cargo test -p bifrost-shortcuts`. New tests in `automation::tests`:
`picker_gives_each_shortcut_its_index_name_colour_and_action_count`,
`the_entity_query_reads_its_subtitles_from_the_picker`, `the_shortcut_in_front_is_the_selected_annotation`,
`a_grid_shows_every_tile_and_an_editor_shows_one_in_front`, `with_no_parameter_the_shortcut_in_front_is_used`,
`with_no_parameter_and_nothing_in_front_the_parameter_is_asked_for`,
`an_explicit_parameter_wins_over_the_shortcut_in_front`, `the_planner_gives_the_same_text_as_planning_on_this_thread`.
The existing `a_missing_shortcut_parameter_is_asked_for` still holds (nothing in front, so the required-parameter error).

Hand check of expected values: `two_saves()` is "Save Battery" (colour 3, one `LowPower` action) and "Save Power"
(colour 2, no actions). The picker's `actions` for index 0 is "1" and for index 1 is "0"; colour "2" matches only
index 1. A plan of "Save Battery" has one step, so its dialog is "Save Battery runs 1 step.". A plan of "Save Power"
fails with "Save Power has no actions".

## REA evidence

- `ev_83d11d67cabda1fd8a41f978481915a2be92efa2f1adab8fc92ed741cacae191`: rendered official page
  https://developer.apple.com/documentation/appintents/appentity (captured for this change; the reference for the
  entity context the onscreen model mirrors).
- `ev_77ba59c186745572fc4ce29ddf4fae76254e40b529c75b6fc1528fb2f72c89de`: ResolvedValueQueryResult (reused from
  `docs/parity/w6-b08.md`).
- `ev_2ffdab560d1f0c64e367e34e83359d0253ad94afa3a3adfdcbfc113e13a1cc26`: AsyncPublisher page
  https://developer.apple.com/documentation/combine/asyncpublisher (reused from `docs/parity/w6-b03.md`).
- `ev_2adfd0924ec4a4ec983b284470797148968f95e323290ff37ea879cfb81e700d`: Actor page (reused from `docs/parity/w6-b02.md`).

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| app-intents-testing | Value query results (ResolvedValueQueryResult) | `springboard/ui/src/value_query.rs` (used by `springboard/apps/shortcuts-desktop/src/automation.rs` `picker`, `entities`) |
| app-intents-testing | Entity annotation and onscreen context | `springboard/ui/src/onscreen_entity.rs` (used by `springboard/apps/shortcuts-desktop/src/automation.rs` `onscreen_context`, `frontmost`; `src/widget.rs` `on_screen`) |
| app-intents | Intent parameters | `springboard/apps/shortcuts-desktop/src/automation.rs` `declared` (optional shortcut parameter) |
| swift | Actors and isolation | `springboard/sdk/src/actor.rs` (used by `springboard/apps/shortcuts-desktop/src/automation.rs` `Planner`) |

Not wired in this round (row stays as it is): combine / "Async publishers" (`springboard/sdk/src/async_publisher.rs`).

## Remaining gaps

- The async publisher is not used by any app. The Shortcuts app has no stream of values to pull from; a useful wire
  needs the shell's run state (a publisher of run progress), which the shell does not send yet.
- The actor only runs the dry-run plan. The run itself still does nothing, so "running without blocking the UI" is
  not true for real runs until the shell has an action runner. The control-socket caller still waits for the plan
  reply; the window does not.
- The onscreen context covers only the Shortcuts window (editor or picker in front). It does not report the content
  of other apps, and the shell does not read it.
- The window's own parameter picker does not use the value query result; only the control socket's entity query does.
- Not built or run in this stage. Cargo.toml and manifest.toml are at 1.4.0; the lead's build updates `Cargo.lock`.
