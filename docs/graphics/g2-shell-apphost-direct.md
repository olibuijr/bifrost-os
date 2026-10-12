# G2: shell side of the direct path (`shell-apphost-direct`)

Part of the graphics core rebuild (docs/graphics-architecture.md, "Switch-over"; docs/graphics-waves.md row 14).
Owner files: `springboard/src/shell/apphost.rs` and the glue in `springboard/src/shell/desk.rs`.

## What is implemented

### Route decision (`apphost.rs`, pure and tested)

- `Route::{Shm, Direct}`. A direct app draws in the window server; the shell makes no window for it and expects no
  frames (`Route::expects_frames`).
- `route_for(direct_env, compositor)`: `Direct` only when the shell runs with `BIFROST_DIRECT=1` **and**
  `/etc/bifrost/compositor` says `bifrost-wm`. Anything else (labwc, no switch, a missing file) is `Shm`.
- `launch_route()` reads the real environment and the session file. `AppHost::spawn_with` decides the route once at
  spawn, stores it on the host, and always sets `BIFROST_DIRECT` to `1` or `0` for the child, so the SDK agrees with
  the shell and a stale value in the shell's environment never leaks into the app.
- Hello carries no direct flag (`ToShell::Hello { sdk_version, app_id }`), so the shell decides from its own launch
  decision instead of what the app says.
- Fallback: a direct app whose SDK finds no Vulkan device or no `bifrost_layer_v1` global sends shared-memory
  `Buffers`. `route_after_message` demotes the host to `Shm` on that message, sets a one-shot flag, and `ext_tick`
  calls `desk_open`, which now opens an ordinary window (the flag is `take_fallback`).

### Window list and focus (`desk.rs`)

- `wayland_foreign_windows`: a direct app's foreign toplevel is matched by its manifest id (the SDK's xdg app id is
  the manifest id, as in Hello), kept in `desk.native_windows` next to native apps, and gets its running state from
  foreign-toplevel. The app is not listed in `linux_apps`.
- `desk_open` for a direct app starts the app if needed (`ext_open`), makes no `Win`, and queues the foreign
  toplevel in `desk.activate`, which the backend turns into `ActivateForeign` (foreign-toplevel activate).
- `desk_close_app` for a direct app sends Quit (`ext_close`), since the shell has no window to remove.
- `foreign_running(slot)`: the Dock's running dot for a direct app is lit by its foreign toplevel.

### Thumbnails (`apphost::thumbnail`, pure and tested)

A shared-memory app shows its last frame. A direct app has no frame in the shell: its switcher card and crash screen
show a screencopy once `capture_ready`, and its icon before that. The decision is implemented; the capture itself
is not (see todo_left).

## How it maps to the contract

- Shell side of the switch-over: the shell stops being the compositor of direct apps. Pixels and input never pass
  through the shell (the SDK's direct runtime is the other half).
- The app↔shell socket keeps control traffic. A direct app still sends its control messages (menus, intents,
  services); only frames are unused.
- The shared-memory path is unchanged for every other app: `Route::Shm` is the default and the only route without
  `BIFROST_DIRECT=1` under `bifrost-wm`.
- Public skeleton signatures are unchanged. Nothing in `apphost.rs` was a `todo!`; the additions are new items.

## Tests

`apphost::route_tests` (6 tests, expected values worked out by hand):
- the route is direct only under `bifrost-wm` with the literal `1` switch;
- a direct app that sends `Buffers` falls back, and any other message keeps the route;
- only the shared-memory route expects frames;
- an app's window is its first toplevel with that app id (whole-id match, no prefix);
- thumbnails: frame, capture or icon by route, frame presence and capture readiness;
- a direct host ignores frames, is demoted by `Buffers` once, and is not demoted again.

Verified: `cargo check --offline -j 2 -p springboard --tests` and
`cargo check --offline -j 2 -p springboard --features desktop-wayland` pass (no new warnings in the owned files;
the `ext_caret_screen` and `dock_rects` warnings were already there). `python3 tools/check-names.py` exits 0.
The tests were not run (build rules); run `cargo test -p springboard route_tests` on the host to execute them.

## What is left (todo_left)

1. Screencopy capture for switcher and crash thumbnails: the shell does not bind `zwlr_screencopy_manager_v1` yet
   (`bare/wayland.rs` is not owned here). `thumbnail` returns `Capture` once `capture_ready`, but nothing sets that.
   Needs a capture request on the Wayland backend and a capture-ready flag per foreign toplevel.
2. Window close for direct apps: the Dock's Quit (`desktop_dock.rs`) matches native windows by `wayland_app_id`, so a
   direct app falls back to `desk_close_app`, which quits it. The right action is `ForeignWindowAction(id, close)`,
   which needs an effect path from `desk_close_app` (`bin/shell.rs` and `desktop_dock.rs` are not owned here).
3. Other `wayland_app_id` matchers still ignore direct apps: `desktop_bar.rs` (the active window's app name, line 113)
   and `switcher.rs` (line 901). They should use the same manifest-id rule as `foreign_running`.
4. The SDK must set its xdg app id to the manifest id on the direct path (`sdk/src/wl/connection.rs`). Until it does,
   no foreign toplevel matches and the direct app's Dock dot and activation do nothing.
5. The window server must report foreign toplevels with the app id and activate requests (`wm-session`).
6. Keyboard focus and the foreground external app (`ext_fg`) are not set for a direct app; keyboard input and the
   shell's chords go to the window server's focused surface. Check this with the input work (`wm-input`).

## contract_changes

None. The skeleton's public signatures are unchanged. One assumption is recorded above: the direct path's xdg app id
equals the manifest id (item 4 of todo_left).

## How to verify on Midget

Midget: Intel HD 530, Mesa ANV, Vulkan 1.4, the session runs labwc unless `/etc/bifrost/compositor` says
`bifrost-wm`.

1. Build the fast profile on the host: `cargo build --offline --profile fast --bin shell --features desktop-wayland`
   (in `springboard/`), then `tools/midget-push-shell.sh --no-build shot.png`.
2. On Midget, put `bifrost-wm` in `/etc/bifrost/compositor`, start the shell with `BIFROST_DIRECT=1`, and check the
   server is up (`python3 tools/midget.py recovery`).
3. Open a direct SDK app (`bifrost pc midget open-app ID`). The shell should make no window for it
   (`bifrost pc midget state` lists it without a window), and its toplevel should appear in the window server.
4. The Dock's running dot for the app is lit only while its toplevel exists. Selecting the app from the Dock or Window
   menu raises its toplevel.
5. Force a fallback: start the app with the Vulkan device hidden (or the SDK's shared-memory path) while the shell is
   `bifrost-wm` with `BIFROST_DIRECT=1`. The shell should open a window for it the first time it sends `Buffers`,
   and the frames should show.
6. Compare with labwc (remove the session file): every app keeps its own window and nothing changes.
