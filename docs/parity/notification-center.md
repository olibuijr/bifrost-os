# Notification Center parity

Framework: Notification Center (https://developer.apple.com/documentation/notificationcenter), macOS 10.10+.
Apple marks the framework deprecated in favour of WidgetKit. Matrix row: `docs/parity-matrix/data/notification-center.json`.

## What was built

Pure models in `springboard/ui/src` (bifrost-ui), std only, no shell wiring yet.

- `springboard/ui/src/widget.rs`
  - `DisplayMode` (compact or expanded, `toggled()` for the More button). Mirrors `NCWidgetDisplayMode`.
  - `Insets` and `margin_insets(proposed, custom)`: the widget accepts the proposed margins or returns its own.
    Mirrors `widgetMarginInsets(forProposedMarginInsets:)`.
  - `Limits`: the column width and the largest height per mode; expanded is never shorter than compact.
    Mirrors `widgetActiveDisplayModeDidChange(_:withMaximumSize:)`.
  - `Controller`: per-widget has-content flag keyed by bundle identifier, with `visible()` filtering a
    widget list in order. Mirrors `NCWidgetController` (`setHasContent(_:forWidgetWithBundleIdentifier:)`).
    A widget that never reported stays visible; that choice is BifrOSt's, not Apple's.
- `springboard/ui/src/widget_list.rs`
  - `WidgetList<T>`: contents, `row_for`, `item_at`, `visible_rows(height)` (rows that fit, never fewer than
    `minimum_visible_rows`, never more than the content), `add_button_visible()`, and `delete(row)` allowed only
    while editing. Mirrors `NCWidgetListViewController` (`contents`, `row(for:)`, `viewController(atRow:)`,
    `minimumVisibleRowCount`, `hasDividerLines`, `editing`, `showsAddButtonWhenEditing`).
    Drawing and the per-row content controller are not modelled.

Each module has unit tests (7 in `widget.rs`, 5 in `widget_list.rs`). Nothing was built or run in this round.

## Official API names mirrored

- `NCWidgetProviding`: `widgetMarginInsets(forProposedMarginInsets:)`, `widgetActiveDisplayModeDidChange(_:withMaximumSize:)`, `NCWidgetDisplayMode` (`compact`, `expanded`).
- `NCWidgetController`: `setHasContent(_:forWidgetWithBundleIdentifier:)`, `default()` (the shell owns the instance instead).
- `NCWidgetListViewController`: `contents`, `row(for:)`, `viewController(atRow:makeIfNecessary:)` (row lookup only), `minimumVisibleRowCount`, `hasDividerLines`, `editing`, `showsAddButtonWhenEditing`.

Official facts used, from the DocC pages read with `tools/parity/docc.py` (the landing page and the four symbols
above) and the rendered landing page.

## REA evidence

- Landing page, rendered: `ev_edff570c97d094b615f72c16d3441d6dd6f11507c79f76b0692f81d37f6758b2`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/notificationcenter`). It shows the
  "Deprecated. Use WidgetKit instead." banner and the four topic groups (widget protocol, controller, search view,
  list view).
- The earlier matrix row cited `ev_a365b0b1626848378c0b955b55ce0bc7fb3c3f9ce03a732a868f5783e4460582`; it is kept
  here for reference only.

## Remaining gaps

- No app-facing SDK type: `bifrost_app` does not expose the widget models, so apps cannot supply a Today widget.
- The models are not used by the Today View: no hide of empty widgets, no display-mode switch, no size limits.
- `widgetPerformUpdate` and the editing callbacks (`widgetAllowsEditing`, `widgetDidBeginEditing`, `widgetDidEndEditing`)
  have no counterpart.
- The search view (`NCWidgetSearchViewController`, `NCWidgetSearchViewDelegate`) is not started.
- The list view has no drawing, no content view controller per row and no delegate.
- The Notification Center column does not show widgets (`springboard/src/shell/notify/center.rs`).
- WidgetKit is not implemented; it is outside this framework.
