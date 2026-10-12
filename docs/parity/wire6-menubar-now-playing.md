# Wire 6: menu bar Now Playing extra

Branch `wire6/menubar-now-playing`, built on `main` (0.5.43). Not built or run: the lead builds once. Builds on
`wire5-now-playing-cc.md` (the Control Center tile and seek) and `wire-svc-now-playing.md` (app sessions and transport
commands). The wire protocol (`springboard/proto`) and the SDK are unchanged.

## What is now wired

- A Now Playing extra in the menu bar. It appears while an app has a published Now Playing session
  (`now_playing::publish`), placed left of the system extras (Control Center, input, Wi-Fi or ethernet, power, CPU and
  RAM readouts) and right of the app menu extras. It disappears when the session is cleared or its owner closes.
- Its menu (a panel of rows under the bar) shows the title, then ` - ` and the artist and album, then the position:
  `1:05 of 3:20` for a finite item, `1:05` for an open item, `Live` for a live item.
- Below a separator, three transport rows: Previous, Play or Pause (the playback state picks the label and the
  command), and Next. A row the owner did not enable is shown as a plain label, so it cannot be chosen. A chosen row
  sends its remote command through `now_playing::send` (the existing service path), and the owner receives a
  `command` event as before.
- The panel model (`panel_for`, `time_text`) is a pure function over `MediaSession` and the service's `Progress`, so
  it is unit tested on the host.

Model reuse: `bifrost_ui::media_session` (`RemoteCommand`, `CommandState`, `MediaSession`), `bifrost_ui::media_content`
(`MediaDuration`, `subtitle()`), and the existing `now_playing::transport_for`, `now_playing::current`,
`now_playing::progress`, `now_playing::send`. No new model and no new crate.

| Piece | Path |
|---|---|
| Panel model: time text, title and position lines, transport rows, tests | `springboard/src/shell/now_playing_menu.rs` (new) |
| Menu enum variant `NowPlaying`, extra placement and label, rows, `Action::Media`, action handler | `springboard/src/shell/desktop_bar.rs` (hooks only) |
| Module declaration | `springboard/src/shell/mod.rs` (one line) |

## How a user reaches it

1. Open an app that publishes a Now Playing session with `play` (and `pause`, `next`, `previous`) enabled. The
   menu bar gains a Now Playing item showing the title, left of the system extras.
2. Click the item. The menu opens with the title, the position and the transport rows.
3. Choose Pause, Play, Previous or Next. The owner app receives the `command` event; the menu closes.

Publishing `stopped` or calling `clear` removes the item. When no app has a session, the bar is unchanged.

## How to verify on the test PC (Midget)

Midget only (no VM). After the lead's build, `bifrost pc midget ota` until `phase=accepted`:

1. `bifrost pc midget state` shows the shell up with no Now Playing item in the bar.
2. Launch an app that publishes a 200 s session with `play`, `pause`, `next` and `previous` enabled, state `playing`.
   The bar shows its title to the left of the system extras, after the app extras.
3. Click the item (`bifrost pc midget tap` on the bar's extra, using the position from `state`). The menu lists the
   title, `0:xx of 3:20`, and three rows. Previous is greyed out only if the app did not enable it.
4. Choose Pause. The app's `service_event` shows `command` `pause`; the menu's next open shows Play.
5. Publish `paused` and `stopped`, then `clear`: the item goes away.
6. `bifrost pc midget screen PATH` and check the item and menu against the bifrost-hig checklist (CHR-01, CHR-04 for the
   translucent menu, TYP-01 for the 12 pt bar label and 13 pt menu text, COL-01 for named colours only).

Host check (the lead runs this once): `cargo test -p springboard now_playing_menu` and
`cargo test -p springboard desktop_bar`.

## REA evidence

No new REA capture this round. The reference behaviour is the one recorded for the matrix rows:

- Now Playing framework page: `ev_b851395bbd5ed4a24947758f6d3c4a5a2f08a78adeca6e13a0413124a2145dc8`.
- Media Player framework page: `ev_1546766457d8bd5255026935cabaef491d1c89baeafb20187fb19940392f1b34`.
- `MediaSession` page and `MPRemoteCommandCenter` page, as listed in `wire5-now-playing-cc.md`:
  `ev_90f8a9f2340f28e60b29a32d79769430d6c9b3d253e85ae6ddf86966b8109fb9`,
  `ev_82305e42f017fed54a4e12b434f47e7b63fc806299c92436c71414c73f54fe42`.

## Remaining gaps

- No artwork. The service track carries no artwork, so the menu has no image. Needs an artwork field on the service
  publish and a size model from `bifrost_ui::media_artwork` (not wired).
- No separate panel with a progress bar and a seek target. The menu shows the position as text; seek by dragging is
  still only on the Control Center tile (wire 5).
- No skip-interval rows (`skip_forward`, `skip_backward` need `bifrost_ui::skip_interval`) and no playback-rate row.
- Stop and feedback commands are not in the menu.
- The position text is taken when the bar is drawn. An open menu does not refresh its position on its own; it updates
  on the next redraw. Not checked on Midget.
- The menu is a plain popup, not the translucent floating panel described in the design rules; the existing bar menus
  use the same popup, so it inherits their material.
- Not yet tested on Midget.
- `docs/bifrost-ui.md` "System services" should note the menu bar consumer of `now_playing::progress` (not edited here).

## Matrix rows

These rows advance (`docs/parity-matrix/data/`; statuses are unchanged, the lead decides after Midget testing):

| Slug | Feature (as written in the matrix json) | BifrOSt path |
|---|---|---|
| now-playing | Play, pause, stop and toggle commands | `springboard/src/shell/now_playing_menu.rs`, `springboard/src/shell/desktop_bar.rs` (`Menu::NowPlaying`, `Action::Media`) |
| now-playing | Next and previous track commands | `springboard/src/shell/now_playing_menu.rs` (Previous and Next rows), `springboard/src/shell/desktop_bar.rs` |
| now-playing | Local sessions: MediaSession and MediaSessionRepresentable | `springboard/src/shell/now_playing_menu.rs` (`panel_for`), `springboard/src/shell/services/now_playing.rs` |
| now-playing | Content types and metadata (MusicContent, PodcastContent, MovieContent, TVShowContent, BookContent, RadioContent, GenericContent, MediaType, MediaDuration) | `springboard/src/shell/now_playing_menu.rs` (title, artist, position from `MediaDuration`) |
| media-player | Remote command center for apps | `springboard/src/shell/now_playing_menu.rs`, `springboard/src/shell/desktop_bar.rs` (menu rows send `now_playing::send`) |
| media-player | Now Playing information | `springboard/src/shell/now_playing_menu.rs` (`panel`) |
