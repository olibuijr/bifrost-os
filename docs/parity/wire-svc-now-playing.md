# Wire: now-playing service (Now Playing for apps)

Branch `wire/svc-now-playing`, built on `parity/integration`. Not built or run yet: the lead builds once.

## What is now wired

Apps publish a Now Playing session (title, artist, album, duration or live, playback state and position, and the
commands they handle). Control Center's media tile shows the newest session. Taps on its transport row go back to the
owning app as `command` events: play or pause (follows the published state), previous and next. A seek goes back as a
`seek` command with `position_ms`. A command the app did not enable is never sent. The session is dropped when its app
closes.

The models are the existing pure ones: `bifrost_ui::media_session::MediaSession` (state, command table, `dispatch`),
`bifrost_ui::media_content::MediaContent` and `MediaDuration`. No new model was written.

| Piece | Path |
|---|---|
| Shell service `now_playing` (`publish`, `clear`, events, pruning) | `springboard/src/shell/services/now_playing.rs` |
| Registration (one line) | `springboard/src/shell/services/mod.rs` |
| SDK client (publish body, `publish`, `clear`, `command_from_event`) | `springboard/sdk/src/now_playing.rs` |
| Control Center media tile (app session first, else the audio service) | `springboard/src/shell/control_center.rs` |

Service call bodies (JSON, the SDK builds them):

- `publish`: `{id?, title, artist?, album?, type?: "audio"|"video", live?: bool, duration_ms?: number,
  state: "playing"|"paused"|"stopped", position_ms?: number, commands?: {name: "enabled"|"disabled"}}`. Command names:
  `play`, `pause`, `stop`, `toggle`, `next`, `previous`, `skip_forward`, `skip_backward`, `seek`.
- `clear`: removes the caller's session when it is the one shown.
- Event `command` to the owner: `{"command": "<name>"}`; `seek` adds `"position_ms"`.

Wire protocol unchanged: the service channel (`ToShell::ServiceCall`, `ToApp::ServiceEvent`).

## How a user reaches it

1. Open an app that publishes a session (none ships this round; the SDK call is `now_playing::publish(ctx, &session)`).
2. Swipe down from the top-right of the status bar to open Control Center, or look at the desktop Control Center
   cluster. The Now Playing tile shows the app's title and artist.
3. Tap play or pause, previous, next. The app receives `command` in `App::service_event`.
4. Closing the app removes the tile.

When no app has published a session, the tile falls back to the audio service track, as before.

## How to verify on the test PC (Midget)

Use Midget only (no VM). After the lead's build and `bifrost pc midget ota` until `phase=accepted`:

1. `bifrost pc midget state` shows the shell is up. Open Control Center with a swipe from the top-right.
2. Launch an app that calls `now_playing::publish` (a test app or the SDK example) with a title and commands
   `play`, `pause`, `next`, `previous` enabled. The tile must show the title within one frame.
3. Tap the tile's play button with `bifrost pc midget tap-window`. The app's log or `App::service_event` must show
   `command` with `pause` (the session was playing).
4. Stop the app: the tile must return to "Not Playing" when the audio service is idle.
5. Screenshot with `bifrost pc midget screen PATH` and check the tile against the bifrost-hig checklist.

Host check: `cargo test -p springboard now_playing` (service and tile logic) and `cargo test -p bifrost-app now_playing`
once the lead runs tests.

## REA evidence

- `MediaSession` page: `ev_90f8a9f2340f28e60b29a32d79769430d6c9b3d253e85ae6ddf86966b8109fb9`
  (https://developer.apple.com/documentation/nowplaying/mediasession; "Publishing media sessions" describes the
  session as the way to show media on the Lock Screen and Control Center).
- `MPRemoteCommandCenter` page: `ev_82305e42f017fed54a4e12b434f47e7b63fc806299c92436c71414c73f54fe42`
  (playback, next and previous, seek and skip command properties).
- Earlier waves, same pages: `ev_9b9f5c2cd3bffc6bc554faf8331787f36a2b1b8b6f75a8e71f8fc512f3457c2f` (MediaSession),
  listed in `docs/parity/w4-now-playing.md`.

Official references used: `mediaplayer/mpremotecommandcenter` (`python3 -I tools/parity/docc.py`) and
`nowplaying/mediasession`.

## Remaining gaps

- Seek from the lock-screen scrubber is not routed: `now_playing::seek` exists (`springboard/src/shell/services/now_playing.rs`,
  marked `allow(dead_code)`) but the lock card in `audiosvc.rs` is outside this round's files.
- Lock screen: the app session is shown only in Control Center. The lock card still shows the audio service track.
- Artwork does not reach the shell (the SDK has the artwork model; nothing is sent).
- Skip-by-interval and playback rate commands are not sent from the tile.
- Enablement is declared by the app, not by the audio service; the tile does not grey out an unsupported direction
  beyond the session's own commands.
- Only the newest publisher is shown; there is no per-app list.
- Not yet tested on the VM or Midget; host unit tests were written but not run (no builds in this round).
- `docs/bifrost-ui.md` "System services" should list `now_playing` (not edited here: outside this round's files).
