# Parity wire-up round 6: Podcasts (player and now-playing)

Branch `wire6/podcasts` from `main` (0.5.43). App `springboard/apps/podcasts-desktop/`, now version 1.7.0 (manifest
and Cargo.toml) with a CHANGELOG entry. Nothing was built or run in this round: the lead builds once on the host, then
the app goes to Midget. The new code was checked by reading only; the hand-worked tests are in `src/transport.rs`.

## What is now wired

1. **Speed menu (0.5x to 2x).** The Speed button in the Now Playing sheet opens a menu above it with seven rows,
   0.5x, 0.75x, 1x, 1.25x, 1.5x, 1.75x and 2x. The current rate's row is selected. Picking a row sets the rate and
   closes the menu; tapping Speed again closes it, and tapping any other control in the sheet closes it too. The
   menu is drawn with the kit card colour (`CARD_BG`), the selection colour (`SELECTION`) and body text (`LABEL`).
   The rate is kept for the session; the engine still plays at 1x because the audio service has no rate command.
   Pure part: `transport::RATES`, `transport::rate_index`, `transport::speed_label`. UI part: `Target::Speed`,
   `Target::Rate(i)`, `speed_menu`, `speed_menu_rect`, `speed_item`, `draw_speed_menu` in `src/podcasts.rs`.
   The old click-to-cycle behaviour (`transport::next_rate`, built on `RateCommand::step`) is removed; the menu
   replaces it.
2. **Mini player and sheet skips, 15 back and 30 forward.** The mini player's back button now skips 15 s (label
   "-15s", it was "-30s"), its forward button still skips 30 s ("+30s"). The sheet's back button is "Back 15" and its
   forward button is "Fwd 30", as before. One pair of constants now serves every caller:
   `transport::BACK_SKIP_S` (15) and `transport::FORWARD_SKIP_S` (30). The Left arrow key and the system's
   skip-backward command also move 15 s; the Right arrow and skip-forward move 30 s. Clamping stays in the
   `skip_interval` model (`transport::skip_target_ms`): no back skip before zero, no forward skip past a known end.
3. **Skip-interval remote commands.** The Now Playing session still declares `SkipForward`, `SkipBackward` and
   `SeekTo` as enabled (`src/media.rs`). When the shell sends `skip_forward` or `skip_backward` (Control Center), the
   app skips with the same constants as the buttons. Nothing new is sent to the service.

## Not wired (gaps, with reasons)

- **Artwork in the Now Playing session.** `springboard/ui/src/media_artwork.rs` (`Artwork`, `choose`, `aspect_fit`)
  is in the SDK, but the feed parser (`src/feed.rs`) reads no `itunes:image` for shows or episodes, and
  `now_playing::publish_body` sends no artwork field (the SDK doc says artwork does not reach the shell yet). The
  Control Center tile keeps the app's mark. Not wired.
- **Per-session playback rate and skip intervals in the service.** `publish_body` has no rate or preferred-interval
  field, so the tile cannot show the chosen speed or the interval the app wants. The skip intervals a session prefers
  (`preferredIntervals`) are fixed in the app. Needs a `springboard/proto` change, which is outside this folder.
- **Engine speed.** The menu changes only what is shown; the audio service plays at 1x. A rate command in the audio
  service is needed.
- **Content types beyond podcast audio.** Podcasts publishes only `audio`; the app plays audio.
  Other content kinds (movie, TV show, music) are not used by this app.
- **Menu bar Playback menu.** The speed choice is in the sheet only. The app has no menu-bar menus yet (the
  `desktop_bar.rs` menu lists would need a new entry and their tests updated), so a Playback menu is left for a later
  round.

## How a user reaches it

1. Open Podcasts, open a show and tap an episode so it plays.
2. The mini player's back button skips 15 seconds and its forward button skips 30 seconds.
3. Tap the mini player (the player area) to open the Now Playing sheet.
4. Tap Speed. A menu opens above the button with 0.5x to 2x; the current rate is selected. Pick a rate; the button
   reads "Speed 1.25x", for example.
5. Control Center's Now Playing tile sends skip commands to the app: back 15 s and forward 30 s.

## How to verify on the test PC (Midget)

Midget is the only test machine (the VM is skipped by request).

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. The app should read version 1.7.0.
2. `bifrost pc midget open-app is.olibuijr.podcasts.desktop`, subscribe to a feed, and play an episode. Wait for the
   clock to move.
3. Skips: tap the mini player's "-15s" and "+30s" with `bifrost pc midget tap-window Podcasts X Y`. Check the clock
   moves 15 s back and 30 s forward (`bifrost pc midget screen /tmp/podcasts-skip.png`). Near the start, back stops at 0:00.
4. Speed menu: open the sheet, tap Speed. Check the seven rows and that the current rate is selected. Pick 1.5x and
   check the button reads "Speed 1.5x" and the menu closes. Tap Speed, then a sheet button other than Speed: the menu
   closes and the other button acts.
5. Control Center: swipe in from the top-right and use the Now Playing tile's skip controls, or send
   `bifrost pc midget key Left` and `Right` with no text field focused. Check the back and forward jumps.
6. Check the speed menu against the bifrost-hig checklist: rows inside the card, nothing clipped at the window bottom
   (the menu opens upward from the Speed button, so a small window may clip the top row; report it if so), and
   `CONTENT_BG` behind the sheet.

Host tests (for the lead, once): `cargo test --offline -p bifrost-podcasts transport` in `springboard/`, then
`cargo test --offline -p bifrost-podcasts` (the compliance snapshot is not changed by this round; the mini player label
is only shown while an episode plays).

## REA and official evidence

- Official DocC read this round (no new REA capture; REA was not started):
  `python3 -I tools/parity/docc.py mediaplayer/mpskipintervalcommand --depth 1` gives MPSkipIntervalCommand and its
  `preferredIntervals` (https://developer.apple.com/documentation/mediaplayer/mpskipintervalcommand);
  `python3 -I tools/parity/docc.py mediaplayer/mpchangeplaybackratecommand --depth 1` gives
  MPChangePlaybackRateCommand and its `supportedPlaybackRates`
  (https://developer.apple.com/documentation/mediaplayer/mpchangeplaybackratecommand).
- REA rendered pages from earlier rounds, reused here (see `docs/parity/wire-app-podcasts.md`):
  - `ev_fff0212db34fb118d740c432b1dde57646e0908d0dd8b24dca89d3bce213e14b` (MPSkipIntervalCommand)
  - `ev_39f7d5ef21336080173f132c02d414ef30bff1832cefb8cda16e820313c3c52f` (MPChangePlaybackRateCommand)
  - `ev_58f114bd69a63d2247ccc391433fda881eee6e7a51ec80d92003e191bdb8a2cd` (MPRemoteCommandCenter)
  - `ev_2365f06731527b2a570c124d447952efe05f7a52cea515c63453ff5c5925fd1b` (MPNowPlayingInfoCenter)
- The 15 s back and 30 s forward intervals are this app's choice, as the podcast convention; the official pages do
  not name them. The menu's rate list (0.5x to 2x) is the one the matrix row asks for.

## Matrix rows

| Slug | Feature (as written in data/<slug>.json) | BifrOSt path | State |
|---|---|---|---|
| now-playing | Skip forward and backward by interval | springboard/apps/podcasts-desktop/src/transport.rs (`skip_target_ms`, `BACK_SKIP_S`, `FORWARD_SKIP_S`) over springboard/ui/src/skip_interval.rs | wired in Podcasts (buttons, keys, remote skip commands); still partial (the service gets no interval) |
| now-playing | Playback rate | springboard/apps/podcasts-desktop/src/transport.rs (`RATES`, `rate_index`) and src/podcasts.rs (speed menu) | wired in Podcasts as a menu; partial (the engine plays at 1x) |
| now-playing | Artwork (Artwork, ArtworkRepresentation, AnimatedArtwork) | none in Podcasts (springboard/ui/src/media_artwork.rs is not sent) | not wired |
| now-playing | Content types and metadata (MusicContent, PodcastContent, MovieContent, TVShowContent, BookContent, RadioContent, GenericContent, MediaType, MediaDuration) | springboard/apps/podcasts-desktop/src/media.rs (`session`, audio type, title, artist, duration) | wired for podcast audio; partial |
| now-playing | Play, pause, stop and toggle commands | springboard/apps/podcasts-desktop/src/media.rs and src/podcasts.rs (`remote`) | wired in Podcasts (earlier wire round for the session; `remote` handles them) |
| avkit | Transport controls and audio mini player | springboard/apps/podcasts-desktop/src/podcasts.rs (mini player: "-15s", play or pause, "+30s"; sheet: Back 15, Play or Pause, Fwd 30, Speed menu, Sleep) | wired in Podcasts; partial |
| media-player | Playback rate and language options | springboard/apps/podcasts-desktop/src/transport.rs and src/podcasts.rs (speed menu) | wired in Podcasts for the rate part only; no language options |
| media-player | Track navigation and seeking | springboard/apps/podcasts-desktop/src/podcasts.rs (`remote`: skip and seek commands) | wired in Podcasts for skips and seek |

## Remaining gaps (summary)

- Artwork in the session (needs a feed image parser and a proto field).
- A rate and a preferred skip interval in the Now Playing session (needs a proto field), and an engine rate command.
- A Playback menu in the menu bar with the speed choices (needs menu-bar work in the shell).
- Chapter navigation (no chapter model read from feeds; see `docs/parity/wire4-podcasts.md`).
