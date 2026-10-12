# Wiring the parity models into Music (1.3.0)

Branch `wire/app-music`. Scope: `springboard/apps/music-desktop/` only.

## What is wired

- **Now Playing session** (`src/music/nowplay.rs`, held by `Music` as `now_playing`). The app keeps one
  `MediaSession` (`bifrost_app::now_playing`, model `springboard/ui/src/media_session.rs`) in step with the audio
  service snapshot. `sync_now_playing` runs on every snapshot and refresh:
  - content: the song (title, artist, album, a finite length from the snapshot's `duration_ms`, else continuous) or
    the station (`MediaDuration::Live`), via `MediaContent` (`media_content.rs`);
  - playback state (`Playing`, `Paused`, `Stopped` with no track) and position (clamped to the length by the model);
  - command table: Play, Pause and TogglePlayPause follow the state, Next is enabled with a following queue item or
    Repeat All, Previous with any track, SeekTo for a seekable song. Stop and the skips stay `Unsupported`.
- **System commands** (`Music::remote_command(name)`): `play`, `pause`, `stop`, `toggle`, `next`, `previous` are
  checked with `MediaSession::dispatch` and then run the same `control` path as the buttons. Pause while paused and
  Play while playing do nothing. Unknown names return false.

Unit tests are in `nowplay.rs` (`cargo test -p bifrost-music nowplay`, run by the lead only).

## How a user reaches it

- Play a song or a station in Music. The session follows it; the Now Playing sheet and the mini player are unchanged.
- The shell's lock card and Control Center still show the audio service's own track. They do not read the app session.

## How to verify on the test PC (Midget)

1. `bifrost pc midget open-app is.olibuijr.music.desktop`, then play a song and confirm the mini player and lock card
   show the same title (this path is unchanged).
2. Once the shell delivers `now-playing` command events (see gaps), send `{"cmd":"pause"}` from the lock card; the
   app should pause, and Pause while already paused should do nothing.
3. Expected: no change to the Music window. The session is internal until gap 1 is closed.

## REA evidence

- `ev_d5370fb464b3447e314774fc1356491a5676e5067701d3305cdf163f9177abab`: MPRemoteCommandCenter page (command table:
  pause, play, stop, toggle, next, previous, repeat, shuffle, rate, seek, skips, position, rating, feedback).
- `ev_408c161760fb23d488499da8066c836a0b6d4ea18ae7a71fe8522afe5e476db0`: MPNowPlayingInfoCenter page (the
  Now Playing info a player sets).
- Official docs read with `tools/parity/docc.py mediaplayer/mpnowplayinginfocenter` and `mediaplayer/mpremotecommandcenter`.

## Remaining gaps

1. **Command delivery.** `bifrost_app::App::service_event` receives `now-playing` events, but the shell never sends
   `command` events (only a protocol test in `springboard/proto/src/rich.rs` does). Music needs
   `fn service_event` calling `remote_command(body.str_field("cmd"))` for `service == "now-playing"`, which needs
   `bifrost-core` as a path dependency (other apps add it the same way). That change touches `Cargo.toml` and
   `Cargo.lock` outside this folder, so it is not made here.
2. **Publishing to the lock card.** The audio service carries no app session, so the session stays local. Closing this
   needs a service channel (a `now-playing` publish) and artwork over it.
3. **Playback rate.** `springboard/ui/src/playback_rate.rs` (`RateCommand`) has no audio service command to drive;
   the service snapshot has no rate field. Not wired: a rate control would do nothing.
4. **Media library filters.** `media_sources.rs` (`LoadOptions`) and `media_changes.rs` (`MediaLibrary`) are not
   wired. Music scans fixed roots (`$HOME/Music`) and has no change observation; a source filter would only wrap the
   roots, so the models were left out.
5. **Skip intervals** (`skip_interval.rs`) and the **song-level artwork sizes** (`media_artwork.rs`) need the service
   to carry interval skips and artwork bytes.
6. Cargo version: `Cargo.toml` is 1.3.0; `Cargo.lock` needs the same bump on the next build (lead).
