# Connecting Music to the system services (1.3.0)

Branch `connect/music`. Scope: `springboard/apps/music-desktop/` only. Not built or run: the lead builds once
(`cargo test -p bifrost-music` for the new tests). Builds the service clients this uses from `parity/integration`:
`now_playing`, `audio-devices`, `dock` (SDK clients in `springboard/sdk/src/`, shell services in
`springboard/src/shell/services/`).

## What is connected

| Service | Music does | Where |
|---|---|---|
| `now_playing` | Publishes the Music session (song or station, state, position, enabled commands) to Control Center's media tile. Receives its `command` events (play, pause, toggle, next, previous) and `seek`. | `src/app.rs` (`publish_step`, `system_command`, `sync_system`), `src/music.rs` (`session`, `remote_seek`), `src/music/system.rs` |
| `audio-devices` | Reads the default output once at start (`list`), then follows `route_changed`. Pauses when the output leaves headphones or Bluetooth for the speaker. | `src/app.rs` (`configure`, `service_reply`, `route_changed`), `src/music/system.rs` (`output_lost`) |
| `dock` | Shows an indeterminate progress bar on the Music Dock icon while the library is scanned. | `src/app.rs` (`dock_step`) |

Badge: not connected. The `dock` badge needs the `notifications` permission, and Music has no badge-worthy count. Music
does not add the permission for it.

## Rules (when calls go out)

- Now Playing is published when the song, the playback state, or the enabled commands change, and when a seek moves the
  position more than 2 s away from where normal playback would be. While a song plays normally nothing is sent. A stopped
  session (no song) is withdrawn with `clear` once. A live station never triggers a seek publish (SeekTo is disabled).
- The Dock progress is sent only when the scan starts or ends.
- `list` is sent once when the window is configured. Route events are only read; nothing is asked on each frame.
- Calls are checked in the tick: no call is made in the frame loop when nothing changed.

## How a user sees it

1. Play a song. Open Control Center (swipe from the top-right of the status bar). The Now Playing tile shows the song
   title and artist. Its play or pause, next and previous buttons act on Music.
2. A seek sent to Music by the shell (the tile's seek, where the shell sends one) moves Music to that position. The lock
   screen scrubber is not routed yet (see gaps).
3. Unplug headphones (or switch off the Bluetooth output) while a song plays. Music pauses.
4. Start Music with a large library: the Dock icon shows a progress bar until the scan has ended.

## How to verify on the test PC (Midget)

Use Midget only. Build the fast profile, push the shell, publish, and update Midget until `phase=accepted` (see the
`bifrost-pc` skill).

1. `bifrost pc midget open-app is.olibuijr.music.desktop`, then play a song from the Songs list.
2. `bifrost pc midget state` lists the `now_playing` and `audio-devices` services. Open Control Center with a swipe from the
   top-right. The tile shows the song.
3. Tap pause on the tile with `bifrost pc midget tap-window` (the tile is shell-drawn; use the tile's screen position from
   `bifrost pc midget screen PATH`). Music pauses; the tile shows paused. Tap play: Music resumes.
4. With a song playing, unplug the headphones (or switch the output off Bluetooth). Music pauses. Tap play on the tile
   to resume.
5. Quit Music: the tile falls back to the audio service track (or "Not Playing" when the service is idle).
6. Relaunch Music on a large `~/Music`; the Dock icon shows a progress bar while the library scans.
7. An older shell answers `no service ...` to these calls. Music then keeps working; only the tile and the Dock progress
   are missing.

Host check (lead, after the build): `cargo test -p bifrost-music` runs the glue tests in `src/music/system.rs` and
`src/app.rs` (`the_dock_shows_progress_only_while_the_library_scan_runs`,
`the_now_playing_tile_is_published_on_change_and_on_a_seek_only`, and the route and reply tests).

## Gaps

- Artwork is not sent: the SDK has the artwork model, but the shell does not take it yet, so the tile shows text only.
- Shuffle and Repeat are not offered to Control Center (the session has no commands for them in the shell).
- Stop and the skip-by-interval commands are not handled; the session does not enable them.
- The route pause uses the default output's id (`headphones`, `bluetooth`, `speaker`). It does not know the device name, so
  two Bluetooth devices are one `bluetooth` output.
- The Music window does not show the output device it plays into; this needs a drawing change outside this round.
- `Cargo.lock` gains the `bifrost-core` edge for `bifrost-music` when the lead builds (the new path dependency).
- `docs/bifrost-ui.md` "System services" is not edited here (outside this app's folder).
- The `dock` progress needs no permission in the shell (only the badge does); verify on Midget.
- The lock card still shows the audio service's track; the app session only reaches Control Center.
