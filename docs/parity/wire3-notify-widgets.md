# Wire 3: Notification Center widgets (search view and relevance)

## Wired

1. **Relevance time clues rank the widget stack.** The widget area of Notification Center
   (`shell/notify/center.rs`, `widgets_timeline::rows`) used to show widgets in the order they were reloaded. Now each
   widget may carry a relevance hint, and the shell orders the rows with `bifrost_ui::relevance::rank`:
   - `{score, duration}`: the widget is eligible for `duration` seconds from its entry's date, with `score`.
   - `{score, at}` or `{score, from, to, closed?}`: the widget is eligible while a `TimeClue` is active. The clue's
     `DateKind` (`default`, `informational`, `scheduled`, from `kind`) moves the score, and the clue's window decides
     eligibility. `closed: true` makes a range (end included); otherwise it is an interval (end excluded).
   - A widget without a hint keeps its place after the ranked widgets, as before.
   - Live activities still follow the widgets, in their existing order.
   The change is in `springboard/src/shell/services/widgets_timeline.rs`: `Hint`, `Hint::item`, `hint_of`,
   `Store::reload_ranked`, `Store::rows`.

2. **Widget search view service.** A widget's search view (the reference `NCWidgetSearchViewController`: a
   description, a placeholder for an empty result list, and results shown as strings) is now stored per app and
   widget kind, and is readable through the service channel. Model: `bifrost_ui::widget_search::WidgetSearch`.
   Blank description or placeholder text counts as no text, and at most 64 results are kept.
   - `search {kind, description?, placeholder?, results:[string]}` replaces the view; returns the result count.
   - `search_rows {kind}` returns `{rows:[string], placeholder, description}`; `placeholder` is set only while there are
     no results.

## How a user reaches it

- Relevance: an app that reloads a widget timeline sends `relevance` with the `reload` call (see the service header in
  `widgets_timeline.rs`). The Notification Center widget area then lists the most relevant widget first. A time clue
  moves its widget to the front while its window is open, and the widget drops back when the window closes.
- Search view: an app calls `search`. There is no shell surface that draws it yet, so a user cannot see it.

## Verify on Midget

Host unit tests cover the ranking and the search service. The build is not run in this round (the lead builds once).

1. Build and push the shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in
   `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`.
2. Open Notification Center and check that the widget area lists the widgets in the order their hints give. This needs
   an app that reloads with `relevance`. Without one, the order is the reload order, as before.
3. Host tests that pin the behaviour: `widgets_timeline::tests::a_higher_score_comes_first_and_an_expired_entry_falls_behind`,
   `a_time_clue_scores_by_its_hint_and_drops_out_when_it_ends`, `a_relevance_object_in_the_reload_call_builds_the_hint`,
   `a_search_view_holds_its_results_and_shows_the_placeholder_when_empty`.

## Reference evidence

- Official pages read with `tools/parity/docc.py` (no REA page capture this round):
  - `relevancekit/relevantcontext`: the Time clues section lists `date(_:)`, `date(_:kind:)`, `date(interval:kind:)`,
    `date(range:kind:)`, `date(from:to:)`, `RelevantContext.DateKind`. The official page says Smart Stack relevance
    applies on watchOS. The shell applies it to the desktop widget area, which the matrix already records as n/a for
    watch-only clues.
  - `notificationcenter`: the Search View section lists `NCWidgetSearchViewController` and
    `NCWidgetSearchViewDelegate`, described as the default search view within a macOS Today widget.
- No REA evidence id was collected.
- The score amounts (kind offsets, point clue duration) are this model's choice. The official pages give no amounts.

## Remaining gaps

- **Search view UI.** There is no Edit Widgets surface or widget gallery in the shell. The search service is ready, but
  nothing draws the search field, the results list or the placeholder. The next step is a small sheet in
  `shell/notify/center.rs` that calls `search_rows`.
- **Relevance from the system.** Only apps supply hints. The shell does not derive hints from its own clocks, calendar
  or sensors.
- **Closed ranges.** A `closed: true` clue is eligible at its end instant. The ranking follows `TimeClue::is_active`, so
  it matches the model.
- **Midget verification** is pending, as is the build and the test run.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| notification-center | Search view (NCWidgetSearchViewController) | springboard/ui/src/widget_search.rs; service in springboard/src/shell/services/widgets_timeline.rs (`search`, `search_rows`) |
| relevancekit | Time clues | springboard/ui/src/relevance.rs; ranking in springboard/src/shell/services/widgets_timeline.rs (`relevance` hint) |
| widgetkit | Smart Stacks and relevance (RelevanceConfiguration, TimelineEntryRelevance, WidgetRelevance) | springboard/src/shell/services/widgets_timeline.rs (`reload` with `relevance`); springboard/ui/src/relevance.rs |
