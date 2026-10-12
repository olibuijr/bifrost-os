# Wire 5: widget options and deep links

## Wired

1. **Widget options (Configurable widgets).** An app defines the options of its widget with the `configure` method
   of `widgets_timeline`: choices (a list of names), flags, and whole numbers with a range (`bifrost_ui::widget_config`).
   The app reads the values in effect with `options` and sets them from its own side with `set_option`. The shell
   draws the options in the Notification Center's Edit Widgets gallery:
   - a widget with options shows an **Options** chip, to the left of Small;
   - Options replaces the gallery with one row per option: the name on the left, its value in `LINK` on the right;
   - a tap on the right half of a row steps it up (a choice moves to its next name and wraps, a flag flips, a number
     goes up by one and stops at its maximum); a tap on the left half steps it down;
   - **Done** returns to the gallery.
   The values are kept across the app's reloads. Defining the same options again keeps the person's values; a
   different list starts at its defaults.
2. **Deep links from widgets (widgetURL).** A `reload` may carry `link: "bifrost-app://<app>/<path>"`
   (`bifrost_ui::widget_link`). A malformed link refuses the reload and changes nothing. A reload without a link
   clears the widget's link. Tapping a widget row in the Notification Center column opens its app and, when the widget
   has a link, sends the app `ToApp::Open { url }` with the link text (the same message the shell uses for URLs). A
   widget with no link opens the app.

Files:
- `springboard/src/shell/services/widgets_timeline.rs`: `Widget.link`, `Widget.config`; `Store::{set_link, link,
  configure, configuration, set_option, step_option}`; methods `configure`, `set_option`, `options`, and the `link`
  field of `reload`; module functions `link`, `configuration`, `step_option`; `Row.widget`, `Listing.configurable`;
  parsing helpers `parameter_of`, `value_of`, `config_error`. Tests:
  `a_widget_link_is_read_on_reload_and_a_link_outside_the_scheme_is_refused`,
  `a_widget_offers_options_once_it_defines_them_and_the_values_survive_a_reload`.
- `springboard/src/shell/notify/center.rs`: `Center.configuring`; `DeskHit::{Options, Param, Widget}`;
  `center_options_layout`, `center_options_hit`, `configuring_count`, `option_at`, `open_widget`,
  `draw_option_rows`; the Options chip in `center_gallery_layout`, its hit test and drawing.
- `springboard/src/shell/workspace.rs`: `desk_open_or_start` is `pub(super)` so the column can open an app.

## How a user reaches it

1. Open Notification Center from the desktop menu bar clock and press **Edit Widgets** at the foot of the column.
2. A widget that defines options shows **Options**. Press it, tap an option row to change its value, then **Done**.
3. Tap a widget row in the column to open its app. If the widget set a link, the app receives it through `Open`.

## Verify on Midget

1. Build and push the shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in
   `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`.
2. Host tests (the lead builds once): `cargo test -p springboard services::widgets_timeline` (the two new tests above,
   plus the existing `Row` assertions, which now carry `widget`).
3. On Midget, open an app that reloads a widget with a `link` and `configure` (for example a test app). Open
   Notification Center, press Edit Widgets. The row shows the Options chip. Press it, tap the rows, check the values
   change, press Done.
4. Tap the widget row in the column. The app comes to the front; with a link, the app's `Open` handler receives the
   link text.

## Reference evidence

- `ev_da50420fcc8be2f31682105b1f7b68fc8b1ab6462f071eb69ee15991828508cb`: AppIntentConfiguration (Apple Developer
  Documentation). Configurable widgets are a configuration with user-editable parameters, shown when the person adds or
  edits the widget.
- `ev_286acf7037881d7f3332746f2817c40b9486f6a25f433ab9361405f99a8ae01f`: widgetURL(_:) (Apple Developer
  Documentation). Sets the URL the containing app opens when the person clicks the widget; a widget has one such
  modifier.

## Remaining gaps

- **Options are shown only in Notification Center.** The Home-screen widget gallery (`widgetgallery.rs`) does not offer
  them yet.
- **Apps read values by polling.** The shell sends no event when a person changes an option; the app calls `options`
  again (typically on its next reload or timeline pass).
- **Only the shell's Open message carries the link.** An app that does not handle `Open` ignores the link. The path is
  not resolved by the shell.
- **Options have no text input or Large layout.** Numbers step by one; there is no free-text or colour option.
- **No Interactivity.** Widget actions (`widget_action`) and intents are not wired; a row tap only opens the app.
- **Not built or run.** The host tests and the Midget check are pending.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| widgetkit | Configurable widgets (AppIntentConfiguration, WidgetInfo) | springboard/src/shell/services/widgets_timeline.rs (`configure`, `set_option`, `options`, `step_option`); springboard/src/shell/notify/center.rs (`draw_option_rows`, `center_options_hit`, Options chip) |
| widgetkit | Deep links from widgets (widgetURL, Link) | springboard/src/shell/services/widgets_timeline.rs (`reload` `link`, `set_link`, `link`); springboard/src/shell/notify/center.rs (`open_widget`) |
| widgetkit | Timeline updates (TimelineProvider, TimelineEntry, Timeline, WidgetCenter reload) | springboard/src/shell/services/widgets_timeline.rs (`reload` carries the link and keeps options across reloads) |
