# Wiring the parity models into Podcasts (is.olibuijr.podcasts.desktop 1.4.0)

Scope: `springboard/apps/podcasts-desktop/` only. The app is now 1.4.0 (manifest and Cargo.toml). Nothing was built or
run for this change; the tests were written by hand and are for the lead to run once on the host, then on Midget.

## What is now wired

| Matrix slug | Feature (data/<slug>.json) | BifrOSt path | Status |
|---|---|---|---|
| now-playing | Skip forward and backward by interval | `springboard/ui/src/skip_interval.rs` (`SkipCommand`) via `src/transport.rs` | Wired in the app: back and forward skips use the model's clamping (no seek before zero, no seek past a known end). |
| now-playing | Playback rate | `springboard/ui/src/playback_rate.rs` (`RateCommand::step`) via `src/transport.rs` | Wired in the app: the speed button steps 0.5x to 2x through the model and wraps. The rate is shown and kept for the session; the engine still plays at 1x. |
| media-player | Playback rate and language options (rate part) | `springboard/ui/src/remote_command.rs` is not used; the app uses `playback_rate.rs` | Same as above. |
| now-playing | Play, pause, stop and toggle commands (in-app form) | `src/podcasts.rs` `key()` | Space toggles play and pause, as a keyboard stand-in for the system's remote command, when no text field has focus. |
| now-playing | Skip (in-app form) | `src/podcasts.rs` `key()` | Left and Right arrows skip back and forward 30 seconds, when no text field has focus. |

## Not wired (gaps, with reasons)

- **Now Playing info to the system** (`media_session.rs`, `media_content.rs`, `media_artwork.rs`): the audio service
  (`springboard/proto/src/svc.rs`) has no command for an app to publish a title, artwork or playback state. Changing it
  is outside this folder. The lock card and Control Center show only what the service knows.
- **Remote commands from the system** (`remote_command.rs` `CommandCenter`, media keys): apps cannot register handlers
  for system transport events, and hardware media keys are not mapped to apps. Only the in-app keys above exist.
- **Playback rate in the engine**: the audio service has no rate command or field. The speed button changes only what
  is shown.
- **Resume positions**: already implemented in the app before this change (`set_resume`, saved every 5 s and on
  suspend). No parity model is involved, so nothing new is wired.
- **Rate in Now Playing and skip-interval state for the lock card**: not offered by the service.

## Where the code is

- `src/transport.rs` (new): `skip_target_ms`, `next_rate`, `speed_label`, `RATES`, `PLAYER_SKIP_S`, `SHEET_BACK_S`,
  with unit tests.
- `src/podcasts.rs`: `skip(backward, interval_s)` replaces the old `skip(delta)`; `Target::Speed` uses `next_rate`; the
  speed label uses `speed_label`; `key()` handles the transport keys when no text field has focus.
- `src/main.rs`: `mod transport;`.
- The speed label reads "Speed 1x" by default, as before, so the existing snapshot goldens are not affected.

## How a user reaches it

1. Open Podcasts (the desktop app), subscribe to a feed, open a show and tap an episode so it downloads and plays.
2. The mini player's back and forward buttons skip 30 seconds and stop at the start or the end.
3. Tap the mini player's Now Playing sheet (the player area) to see Back 15, Play or Pause, Fwd 30, Speed and Sleep.
4. Tap Speed to step 0.5x, 0.75x, 1x, 1.25x, 1.5x, 1.75x, 2x, then back to 0.5x.
5. With no text field focused, press Space to play or pause, and the Left or Right arrow to skip.

## How to verify on the test PC (Midget)

Midget is the only test machine (the VM is skipped by request).

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Confirm the app version is 1.4.0.
2. `bifrost pc midget state` shows the Podcasts window. Open an episode with `bifrost pc midget tap-window Podcasts X Y`
   (window-local pixels). Wait for playback to start.
3. Skips: tap the mini player's back and forward buttons. Near the start, back should land at 0:00. Near the end,
   forward should stop at the end. Check the clock with `bifrost pc midget screen /tmp/pod.png`.
4. Speed: open the Now Playing sheet, tap Speed repeatedly. The label should step 1x, 1.25x, ..., 2x, 0.5x. Check with
   a screenshot.
5. Keys: with no text field focused, send Space (play or pause) and the Left and Right arrows (skip 30 seconds).
   Check the clock.
6. Resume: pause, close the app, reopen it, and check the episode resumes near where it stopped (existing behaviour).

Host tests (for the lead, once): `cargo test --offline -p bifrost-podcasts transport` in `springboard/` (the unit tests
in `src/transport.rs`), and the app's compliance gate `cargo test --offline -p bifrost-podcasts` (snapshots unchanged).

## REA evidence (official documentation, rendered by Chromium through REA)

- `ev_fff0212db34fb118d740c432b1dde57646e0908d0dd8b24dca89d3bce213e14b`: MPSkipIntervalCommand
  (https://developer.apple.com/documentation/mediaplayer/mpskipintervalcommand). A forward skip of 30 seconds at 2:30
  jumps to 3:00 without playing the skipped content.
- `ev_39f7d5ef21336080173f132c02d414ef30bff1832cefb8cda16e820313c3c52f`: MPChangePlaybackRateCommand
  (https://developer.apple.com/documentation/mediaplayer/mpchangeplaybackratecommand). Exposes `supportedPlaybackRates`.
- `ev_58f114bd69a63d2247ccc391433fda881eee6e7a51ec80d92003e191bdb8a2cd`: MPRemoteCommandCenter
  (https://developer.apple.com/documentation/mediaplayer/mpremotecommandcenter). Play, pause, stop, toggle and track
  commands on the shared centre.
- `ev_2365f06731527b2a570c124d447952efe05f7a52cea515c63453ff5c5925fd1b`: MPNowPlayingInfoCenter
  (https://developer.apple.com/documentation/mediaplayer/mpnowplayinginfocenter). The app sets Now Playing info and
  playback state.

The clamping rule (no seek past a known end, no seek before zero) and the rate tie rule are this repo's own choices,
as the models say; the official pages do not state them.
