# Wire 2: Dock and Notification Center

## Wired

1. **Calendar Dock tile shows the current date.** The package icon
   (`springboard/apps/calendar-desktop/icon.png`) is a fixed picture that reads "Tue 1", so it was wrong on every
   other day. The shell now repaints the tile's paper and draws today's weekday (red) over the day number (black),
   using the shell's local `info.wday` and `info.mday`. The change is in `springboard/src/shell/desk.rs`:
   `calendar_tile_face`, `calendar_face_text`, and the call in the dock tile loop of `draw_desk_dock`.
   The Calendar app's Dock badge (`dock` service, `calendar/connect.rs`) is unchanged; it still carries the day number.

## How a user reaches it

Open the Dock on the desktop. The Calendar tile shows the weekday and day of today. It refreshes when the shell
info changes (minute or day rollover).

## Verify on Midget

1. Build and push the shell: `cargo build --offline --profile fast --bin shell --features desktop-wayland` in
   `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`.
2. `bifrost pc midget screen PATH` and check the Calendar tile reads today's weekday and day.
3. Unit test `calendar_tile_tests::the_calendar_tile_shows_the_day_it_is_not_a_fixed_picture` covers the text with a
   fixed date (Monday 12 October 2026 gives "Mon" and "12").

Visual check still needed: the text sizes (0.2 and 0.5 of the tile height) and vertical positions were set by hand
and must be checked on Midget at the default dock size and at the magnified size.

## Reference evidence

None collected in this round. No Apple documentation was consulted; the change follows the app's own intent (the
Dock shows the date).

## Remaining gaps

- Notification Center widget search view (`ui/src/widget_search.rs`) is not wired. Not started.
- Relevance time clues (`ui/src/relevance.rs`) for widget stack ordering are not wired. Not started.
- Midget visual verification is pending.
- Build and tests have not been run (the lead builds once).

## Matrix rows

No matrix row slugs were checked against `docs/parity-matrix/data/` in this round, so no row table is given.
