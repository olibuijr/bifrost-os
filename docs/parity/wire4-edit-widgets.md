# Wire 4: Notification Center Edit Widgets

## Wired

1. **Edit Widgets gallery in the Notification Center column.** A capsule at the foot of the desktop Notification Center
   column opens a gallery in the column's place. It lists every widget an app has reloaded (the widget search view's
   description is shown under its name when the app set one). Each row has:
   - an add (+, systemBlue) or remove (-, label colour) disc that puts the widget in the column or takes it off;
   - Small and Large chips for the widget's size in the column (Large is the taller 96 pt row, Small 58 pt).
   Done returns to the column. The gallery opens on the widgets each time the column is shown.
2. **Placement and size are kept in the service.** `services::widgets_timeline` keeps `placed` and `large` per
   (app, kind). A widget is placed and Small when an app first reloads it. A reload keeps the person's choice.
   The column draws only placed widgets (`Store::rows`). The Row carries `large`.
3. **Search view subtitle.** The gallery row shows the widget's search view description (`search`, from
   `bifrost_ui::widget_search`), so the search view's text is visible to the person.

Files:
- `springboard/src/shell/services/widgets_timeline.rs`: `Row.large`, `Listing`, `Store::set_placed`, `Store::set_large`,
  `Store::listing`, module functions `gallery`, `place`, `resize`; `rows` honours placement and size.
- `springboard/src/shell/notify/center.rs`: `Center.editing`, `DeskHit::{EditWidgets, Done, Toggle, Size}`,
  `center_edit_rect`, `center_gallery_layout`, `center_gallery_hit`, `draw_desk_gallery`; the column's rect and draw
  include the Edit Widgets capsule; `center_widget_rects` uses the Large height.

## How a user reaches it

Open Notification Center from the desktop menu bar clock. At the foot of the column, above the dock, press **Edit
Widgets**. Press the disc to add or remove a widget, the S or L chip to choose its size, and **Done** to return.

## Verify on Midget

1. Build and push the shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in
   `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`.
2. Use an app that reloads a widget timeline (`widgets_timeline` `reload`). Open the Notification Center from the menu bar
   clock and check the widget row appears.
3. Press Edit Widgets, then the widget's remove disc. The row leaves the column when you press Done. Add it back with the
   add disc.
4. Press the Large chip. The row grows to the taller height. Press Small to return it.
5. Host tests pinning the store behaviour (not run in this round; the lead builds once):
   `services::widgets_timeline::tests::a_widget_taken_off_in_the_gallery_leaves_the_column_and_comes_back`,
   `a_reload_keeps_the_choice_made_in_the_gallery`,
   `a_large_widget_is_a_taller_row_and_the_gallery_shows_the_search_description`,
   `a_gallery_row_without_an_entry_on_screen_is_named_by_its_kind`.
   Two earlier `Row` assertions (`the_entry_on_screen_follows_the_clock_and_is_per_app_and_kind` and a live activity test)
   now carry `large: false`.

## Reference evidence

- Reference behaviour is the matrix row `notification-center` / Widget editing and ordering (cited in
  `docs/parity-matrix/data/notification-center.json`), whose evidence is
  `ev_a365b0b1626848378c0b955b55ce0bc7fb3c3f9ce03a732a868f5783e4460582` (the Notification Center landing page).
- No new REA evidence or docc read was collected in this round. The gallery's Small and Large sizes follow the shell's
  own widget sizes, not an official page. Treat the size choice as this model's design.

## Remaining gaps

- **No scrolling in the gallery.** Rows that do not fit above the dock are not reachable from the gallery.
- **Placement is in memory.** The choices reset when the shell restarts (the apps re-reload their widgets). Saving them
  to a file like `today.ini` is not done.
- **Large shows the same two lines.** The taller row does not yet show more text; it only gives room.
- **No app-facing API.** Apps cannot set a widget's size or placement; the gallery is shell-side.
- **Not built or run.** Host tests and the Midget check are pending.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| notification-center | Widget editing and ordering | springboard/src/shell/notify/center.rs (Edit Widgets gallery, `gallery_item`, `center_gallery_layout`); springboard/src/shell/services/widgets_timeline.rs (`place`, `resize`) |
| notification-center | Search view (NCWidgetSearchViewController) | springboard/src/shell/notify/center.rs (gallery row shows the description); springboard/src/shell/services/widgets_timeline.rs (`listing`) |
| widgetkit | Home-screen placement and gallery (Add Widget, edit, resize, remove) | springboard/src/shell/notify/center.rs (Notification Center gallery: add, remove, Small or Large); springboard/src/shell/services/widgets_timeline.rs (`set_placed`, `set_large`) |
