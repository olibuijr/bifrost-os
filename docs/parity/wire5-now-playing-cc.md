# Wire 5: Now Playing in Control Center (progress and seek)

Branch `wire5/now-playing-cc`, built on `main` (0.5.42). Not built or run: the lead builds once. Builds the wiring
on the earlier `wire-svc-now-playing` round (see `docs/parity/wire-svc-now-playing.md`), which already routes app
sessions and transport taps.

## What is now wired

- The Now Playing tile on the desktop Control Center plate shows a progress bar for an app's published session when the
  item is finite. The bar is a thin strip near the bottom edge of the plate.
- The position moves on while the session plays. The service stores when the position was last published and the
  tile adds the time since then (`extrapolate` in `services/now_playing.rs`). A paused or stopped session holds its
  position, and the position never passes the length.
- Tapping the bar seeks the session. The shell sends a `seek` command with `position_ms` to the owner (the existing
  `now_playing::seek`, no longer `dead_code`). The bar moves at once, so it does not wait for the app to publish.
  A session whose owner did not enable `seek`, or an open or live item, gives no seek target and the bar does not
  respond.
- The `seek_length_ms`, `progress` and `scrub_target_ms` helpers are the only new service surface. The service
  channel and the SDK are unchanged.

| Piece | Path |
|---|---|
| Position extrapolation, seek optimism, progress and seek accessors | `springboard/src/shell/services/now_playing.rs` |
| Progress bar drawing, scrub touch (`Touch::Scrub`), `media_bar`, `scrub_fraction` | `springboard/src/shell/control_center.rs` |

No wire protocol change (`springboard/proto` untouched). No SDK change. No matrix or shared-doc edits.

## How a user reaches it

1. Open an app that publishes a finite Now Playing session (`now_playing::publish` with `duration_ms`).
2. Open Control Center (swipe from the top-right of the status bar, or use the desktop cluster). The Now Playing plate
   shows the app's title and artist, and a bar along its bottom edge fills as the item plays.
3. Tap the bar at a point along its length. The app receives `command` `seek` with `position_ms` in
   `App::service_event`, and the bar jumps there.

When no app has published a session, the plate shows the audio service track as before. Its own scrubber (in
`audiosvc.rs`) is unchanged.

## How to verify on the test PC (Midget)

Midget only (no VM). After the lead's build, `bifrost pc midget ota` until `phase=accepted`:

1. `bifrost pc midget state` shows the shell up. Open Control Center with a swipe from the top-right.
2. Launch an app that publishes a 200 s session with `play` and `seek` enabled, and state `playing`. The bar should
   fill from the left and move forward over a few seconds without a republish.
3. Tap the bar at its middle with `bifrost pc midget tap-window APP X Y` (window-local px). The bar should jump to about
   half. The app's `service_event` should show `command` `seek` with `position_ms` near 100000.
4. Publish `paused`: the bar must stop moving. Publish `stopped` and clear: the plate returns to the audio service state.
5. `bifrost pc midget screen PATH` and check the plate against the bifrost-hig checklist (bar inset from both sides,
   clear of the transport row, `CONTENT_BG` not involved).

Host check (the lead runs this once): `cargo test -p springboard now_playing` for the service and the plate tests.

## REA evidence

No new REA capture this round. The reference behaviour is the one recorded in `docs/parity/wire-svc-now-playing.md`:

- `MediaSession` page: `ev_90f8a9f2340f28e60b29a32d79769430d6c9b3d253e85ae6ddf86966b8109fb9`.
- `MPRemoteCommandCenter` page (seek and skip properties): `ev_82305e42f017fed54a4e12b434f47e7b63fc806299c92436c71414c73f54fe42`.
- Earlier: `ev_9b9f5c2cd3bffc6bc554faf8331787f36a2b1b8b6f75a8e71f8fc512f3457c2f`, listed in `docs/parity/w4-now-playing.md`.

## Remaining gaps

- The menu bar Now Playing item is not built. The menu bar has no media menu extra (`desktop_bar.rs` menu extras and the
  Settings Menu Bar list would both need a new entry, and their tests would need updating). Next round.
- Dragging the bar is not supported; only a tap seeks. The Control Center expanded Now Playing panel draws the bar but
  does not take a seek touch.
- The lock-screen scrubber (`audiosvc.rs`) still does not route to app sessions.
- Skip by interval and playback rate are not sent from the tile. Repeat and shuffle, feedback and artwork are not
  wired here.
- The position is extrapolated from the publish time, so the bar can drift if the app's clock or buffering differs
  from the shell's. The app republishes on each state change to correct it.
- Not yet tested on Midget.
- `docs/bifrost-ui.md` "System services" should note the `seek_length_ms` and `progress` helpers (not edited here).

## Matrix rows

Rows this round advances (`docs/parity-matrix/data/`; statuses are unchanged, the lead decides after Midget testing):

| Slug | Feature (as written in the matrix json) | BifrOSt path |
|---|---|---|
| now-playing | Seek to position and scrubbing | `springboard/src/shell/control_center.rs` (`Touch::Scrub`, `media_bar`), `springboard/src/shell/services/now_playing.rs` (`seek`, `seek_length_ms`) |
| now-playing | Playback state snapshot | `springboard/src/shell/services/now_playing.rs` (`extrapolate`, `Store::progress`) |
| media-player | Track navigation and seeking | `springboard/src/shell/control_center.rs`, `springboard/src/shell/services/now_playing.rs` |
| now-playing | Local sessions: MediaSession and MediaSessionRepresentable | `springboard/src/shell/services/now_playing.rs` (unchanged this round) |
