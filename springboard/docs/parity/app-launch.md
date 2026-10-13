# App launch speed and feedback (round 75, slice app-launch)

Shell side of starting an installed app: `springboard/src/shell/apphost.rs` (`ext_open`, `launch_failed`,
`ext_tick`) and `springboard/src/shell/desk.rs` (`Desk::close`, `desk_open`).

## What was already there

- Dock launch bounce: `desk_open` starts it (`dock_start_bounce`) whichever way the app opens; it ends when the
  app's first window maps (`dock_tick`), or after `BOUNCE_MAX_S` (10 s).
- Placeholder until the first frame: `ext_draw` fills the window black until a frame arrives (no icon, see below).
- Dock progress and attention bounce per app (`TileState`, `dock` service).
- Session restore replays window frames at login (`session.rs`, `session_restore`).

## What this change does

- Launch failure alert: when the spawn fails in `ext_open`, or an app exits before its `Hello` (not counting an
  incompatible host, which already shows "This app needs an update"), `launch_failed` posts a notification from
  that app ("<Name> could not be opened"). The notification goes through the normal delivery model, so an app muted
  in Settings > Notifications lists it without a banner. The app's launch bounce is removed at once.
- Reopen at last size: `Desk::close` records the outer frame of each closed window in `closed_sizes` (the restore
  frame when zoomed). `desk_open` uses it for a new window of the same app in this session. A fixed manifest
  `window_size` still wins.

## Not done (needs a decision or Midget checking)

- Warm apps after close: `ext_close` still sends Quit. Keeping a suspended process per app needs a memory budget
  and changes the quit and `DidTerminateApplication` semantics, so it is left for the lead.
- Placeholder with the app icon: `icon_image` decodes the icon file on every call, so drawing it per frame would
  cost time during the launch. Needs a cached icon first.
- Size across restarts: the session file already stores frames, so a reopen after a shell restart is covered by
  session restore; the in-session memory is only the `closed_sizes` map.
- Midget check: the alert banner placement while the screen is locked, and the bounce stop on a failed spawn.
