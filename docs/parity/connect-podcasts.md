# Connecting Podcasts (is.olibuijr.podcasts.desktop 1.4.0) to the system services

Branch `connect/podcasts`, from `parity/integration`. Scope: `springboard/apps/podcasts-desktop/` only (plus this note).
Nothing was built or run for this change: the lead builds once on the host, then tests on Midget. The app's version stays
1.4.0 (the round's bump, not repeated here).

Services used: `now_playing` (SDK `bifrost_app::now_playing`, shell `springboard/src/shell/services/now_playing.rs`) and
`dock` (SDK `bifrost_app::dock_tile`, shell `springboard/src/shell/services/dock_tile.rs`). Notification scheduling is not
used (see gaps).

## What is connected

| Service | Matrix slug (row) | What the app does | Code |
|---|---|---|---|
| now_playing `publish` / `clear` | now-playing ("Local sessions: MediaSession"; "Now Playing information") | Publishes the episode being played or paused: title, show as artist, length (or continuous when unknown), playing or paused, and the commands it handles (play, pause, toggle, stop, skip back and forward, seek). Clears it when playback ends or stops. | `src/media.rs` (`shown`, `session`, `publish`), `src/podcasts.rs` (`sync_services`) |
| now_playing `command` events | now-playing ("Play, pause, stop and toggle commands"; "Skip forward and backward by interval") | Taps on the Control Center tile reach the app: play, pause, toggle, stop, skip forward and back 30 s, and seek to a position. Acted on only for an episode the audio service has confirmed. Next and previous are not declared, so the tile does not offer them. | `src/main.rs` (`service_event`), `src/podcasts.rs` (`remote`) |
| dock `progress` | appkit-user-interface ("Menus, Cursors, and the Dock") | While an episode downloads, the Dock icon shows a bar: a sweep until the size is known, then the whole percent. Cleared when the download ends or fails. | `src/media.rs` (`dock_bar`, `send_bar`), `src/podcasts.rs` (`sync_services`) |
| dock `bounce` (informational) | appkit-user-interface ("Menus, Cursors, and the Dock") | When a download finishes, the icon bounces once. The shell ignores the call while Podcasts is in front. | `src/podcasts.rs` (`sync_services`, `bounce_ready`) |

Supporting changes in the app:

- `src/media.rs` (new): the pure models for what each service shows, the SDK calls that send them, and unit tests.
- `src/main.rs`: `tick` calls `sync_services`; `service_reply` consumes the app's replies; `service_event` takes shell
  commands.
- `Cargo.toml`: adds `bifrost-core` (the `service_event` signature names its `Json` type, as the Assistant app does).
- `CHANGELOG.md`: four plain-language lines under 1.4.0.

## Call discipline

- Services are called only when what they show changes: the Now Playing session is compared with the last one sent, and
  the Dock bar with the last one sent (so a download sends about one call per whole percent, not per chunk).
- Nothing is sent per frame. `sync_services` runs in `tick`, which the loop calls on its own wake-ups (the playback clock
  and downloads), and each call is a comparison that does nothing when nothing changed.
- Replies are consumed by `service_reply` and never retried. A reply that says "no service" (an older shell) changes
  nothing in the app, so Podcasts keeps working without these services.
- Pending request ids are kept in a list of at most 32; the oldest is dropped beyond that.

## How a user sees it

1. Subscribe to a show and tap an episode. The download shows as a bar on the Podcasts Dock icon, and the icon bounces
   once when it finishes if Podcasts is not in front.
2. While the episode plays, open Control Center (the Now Playing tile). It shows the episode title, the show name and the
   length. Tapping play or pause, stop, skip back or forward, or seeking on the tile acts on the episode in Podcasts.
3. Pausing or resuming in Podcasts, or the episode ending, updates the tile. The tile clears when playback stops.

## How to verify on the test PC (Midget)

Midget is the only test machine.

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Confirm Podcasts is 1.4.0 with
   `bifrost pc midget state`.
2. Open Podcasts with `bifrost pc midget open-app is.olibuijr.podcasts.desktop` (or from the Dock), subscribe to a feed, and
   tap an episode. Take `bifrost pc midget screen /tmp/pod-dl.png` during the download: the Dock icon shows a bar that
   fills (a sweep first if the server sent no size).
3. When it finishes and the app is in the background (switch to another window first), the Podcasts icon bounces once.
4. Playing: open Control Center with a swipe from the top-right. The Now Playing tile shows the episode title and the show
   name, not the file name.
5. Tap pause on the tile: the episode pauses in Podcasts, and the tile shows the paused state. Tap play, then skip forward
   and back on the tile: Podcasts moves 30 seconds each way. Check the clock with a screenshot.
6. Stop from the tile (or from the app's Stop control): the tile clears.
7. Check the tile and the Dock bar against the bifrost-hig checklist (CHR, COL, TYP rules) with a screenshot.

Host tests (for the lead, once, with the cargo commands given in the task): `cargo test --offline -p bifrost-podcasts media`
(the unit tests in `src/media.rs`), then the app's compliance gate `cargo test --offline -p bifrost-podcasts` (the golden
snapshots must not change: nothing in the window was changed).

Cargo.lock: the `bifrost-podcasts` entry in `springboard/Cargo.lock` needs `bifrost-core` added. That file is outside this
app's folder, so it was not edited here; the lead's offline build updates it.

## Gaps

- **Notification schedule: not connected.** Podcasts has no new-episode or resume reminder today, and adding one needs a
  setting and an owner decision. The shell service (`notification-schedule`) is ready when that decision is made.
- **Playback rate on the tile.** The tile does not carry the rate: the now_playing session has no rate field, and the audio
  service has no rate command. The speed button keeps working in the app only (`docs/parity/wire-app-podcasts.md`).
- **Position on the tile.** The session is published when the state changes (start, pause, resume, end, length known),
  not every second, so the tile's position is what it was at the last change. The shell has no position updates from an
  app, and a per-second call is avoided on purpose. Matrix row to revisit: now-playing "Local sessions".
- **Artwork.** The session has no artwork: the app has no artwork service call yet, and the SDK notes say artwork does not
  reach the shell.
- **Next and previous.** Not declared, since Podcasts has no queue. The shell does not offer them on the tile.
- **Remote commands from keys and other apps.** Hardware media keys are not mapped to apps, so only the tile's taps reach
  Podcasts (the in-app Space and arrow keys from 1.4.0 still work).
- **Badge.** The Dock badge (new episodes count) is not set: the app does not track unplayed episodes. The badge also
  needs the `notifications` permission, which the manifest does not declare.
- **Matrix rows.** This note does not change `docs/parity-matrix/data/`. The rows named above stay `partial` until the
  features work on Midget; the lead promotes them then.
- **Version.** `min_shell` stays 0.4.84. An older shell answers "no service" and the app runs without the tile and bar, so
  no bump is needed for this change.
