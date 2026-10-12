# WidgetKit parity, wave 3: widget links, widget interactivity, widget accessibility

Three pure Rust models in `springboard/ui/src` (bifrost-ui), one per gap row below. They add the data side of
the WidgetKit features. None of them is wired into the shell or an app yet: the shell still draws its own tiles
and no app can supply a widget. Nothing was built or run for this round (no cargo, no tests run).

## What was built

| Module | Mirrors (official API) | What it does |
|---|---|---|
| `springboard/ui/src/widget_link.rs` | `widgetURL(_:)`, `Link` in a widget | `Link` parses and validates `bifrost-app://<app>/<path>`. Paths are normalised (empty segments dropped), `.`, `..`, `?`, `#` and whitespace are refused. `to_url` writes the form `parse` reads back. |
| `springboard/ui/src/widget_action.rs` | `Button`, `Toggle`, AppIntent in a widget | `Action` (open a link, run an intent by id, set a toggle), `Hit` regions and `hit_test` (the last region under a point wins, right and bottom edges are exclusive). `Toggle` shows the new state at once, ignores taps while a change is pending, and rolls back on failure. |
| `springboard/ui/src/widget_ax.rs` | Accessible descriptions of widgets (`accessibilityLabel`, value, hint) | `WidgetAccessibility` joins label, value and hint with ", " (blank parts dropped, decorative widgets silent). `combine` joins the phrases of several widget parts with ". ". |

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`. Each has a
`#[cfg(test)] mod tests` with hand-worked expectations.

## Official API evidence

- `widgetURL(_:)` (SwiftUI view modifier): https://developer.apple.com/documentation/swiftui/view/widgeturl(_:)
  REA `ev_8923394c63d1d53529296671da60e0a1d8b5741a69b93ab0c34289e7fb51a627`. The page says the modifier sets the
  URL opened in the containing app when the widget is clicked, and that a widget supports one `widgetURL`
  modifier in its hierarchy.
- Linking to specific app scenes (article): https://developer.apple.com/documentation/widgetkit/linking-to-specific-app-scenes-from-your-widget-or-live-activity
  REA `ev_9b019fd016fc988c3bf68a2f8b6807c0a87c2a266f331d1cae49800dd4e00c4c`. A `Link` control targets its own URL, other taps use the `widgetURL` one, and a widget with no URL opens the app with a user activity.
- Interactivity (article, DocC title "Adding interactivity to widgets and Live Activities"): listed by
  `python3 -I tools/parity/docc.py widgetkit`. The article page itself was not re-rendered in this round.
- Accessible descriptions (article, DocC title "Adding accessible descriptions to widgets and Live Activities"):
  listed by `python3 -I tools/parity/docc.py widgetkit`. REA `ev_fbe496ed1350e1ea4ff267984adf8f8ffb213aeb6d0ec6d1016049bbf03db459`
  rendered a "page can't be found" response for the guessed URL, so its evidence is not usable. The DocC listing is the
  only source for that article.

## Matrix rows closed

Status for every row: `partial` (the data model exists, the system does not yet use it).

| Framework | Feature (as in data/widgetkit.json) | Status | BifrOSt path |
|---|---|---|---|
| WidgetKit | Interactivity in widgets (Button, Toggle, AppIntent in widget) | partial | springboard/ui/src/widget_action.rs |
| WidgetKit | Deep links from widgets (widgetURL, Link) | partial | springboard/ui/src/widget_link.rs |
| WidgetKit | Accessibility descriptions for widgets | partial | springboard/ui/src/widget_ax.rs |

## Not done

- No wiring into `springboard/src/shell/today.rs` (tile taps still open fixed destinations) or into the SDK.
- No app-facing registration of widgets, so the models have no caller yet.
- No tests have been run. The unit tests are written but not executed.
