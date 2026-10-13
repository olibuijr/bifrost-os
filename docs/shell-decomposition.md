# Shell decomposition: updates without a shell restart (plan, 2026-10-11)

Goal (owner): move as much as possible out of the `springboard` shell binary, so each component updates and reloads on its own
without restarting the shell. Measured 2026-10-13 (shell 0.7.25): about 149k lines in `src/shell` plus about 74k elsewhere in `src/` (bare, media, files,
deildu, gpu, net, term). It was 113k and 75k when this plan was written. A shell update restarts the whole session.

## What a restart costs today

- A shell exit ends the session: `packaging/arch/wayland/session` waits on the shell and the window server together (`wait -n`) and
  its cleanup stops the compositor, so every Wayland client goes down with it.
- Apps are children of the shell under `bwrap --die-with-parent` (`src/bare/app_policy.rs`). SDK app windows are shell surfaces
  fed over an inherited socketpair (`shell/apphost.rs`), so they cannot outlive the shell.
- Only native app ids are reopened afterwards (`open-native-windows.json`, `shell/desk.rs`). In-app state is lost.
- Package updates of a running app never relaunch it: `apphost::is_outdated` compares unresolved `current/` paths and does
  not fire (`apphost.rs` `scan` and `ext_open`; the keyboard provider has the same issue). `docs/apps.md` says the app
  gets `Quit`; it does not.
- No privileged channel exists: everything system-level (Wi-Fi, Bluetooth, brightness, power, prefs, dock, appearance, OTA)
  runs inside the root shell through `Effect` (`shell/mod.rs`). `kind = "system"` packages get no extra rights beyond `log-store`.
- The service supervisor (`bifrostd`, with `audiod`, `netd` and `updated`; `springboard/docs/services.md`) hot-swaps
  services on a version change, but nothing installs or starts it on PC.

## Ownership

- `bifrost-wm` (Sol's lane): `springboard/wm/**`, `bare/wayland.rs`, `gpu/wl.rs`, `packaging/arch/wayland/session` and `session-policy`.
  `wm/src/boot.rs` includes `src/bootsplash.rs` by path; do not move it without telling the wm owner.
- This plan: everything else in `springboard/src`, `apps/`, `proto`, `sdk`.

## Phases

0. **Survive a shell restart.**
   - wm side: the bifrost-wm session supervisor restarts only the shell, and the compositor keeps outputs, lock and windows. Today `session` restarts `bifrost-wm` itself (safe mode after three quick exits) but a shell exit still ends the session.
   - Shell side:
     - apps lose `--die-with-parent` and get a reaper or cgroup owner (`bifrost-apps` cgroup).
     - SDK apps reconnect to a named socket (the contract comes with the session).
     - The window list, positions and spaces are written to a hand-off file.
   - Fix `is_outdated` so an updated app relaunches.
1. **Delete what the PC build never reaches.**
   - WoW: removed (branch `chore/remove-wow`).
   - The tablet home edit, jiggle, folders, page edit, widget gallery, App Library and Split View code;
     `window_fx.rs`; the sensor half of `rotate.rs`.
   - Make the Browser package a preinstalled system app, then delete the in-shell browser fallback (`shell/browser`, 31k
     lines, plus html5ever, selectors, taffy and boa).
2. **Built-in apps become packages**, easiest first: Magnifier (needs V4L2 in `app_policy`), Camera (needs a launch-by-id
   intent; saves to the Photos folder), Deildu (split the `scroll`/`http`/`jobs` library that about 15 packages use),
   Photos (the library becomes shared files, since screenshots and Today read it). The library code that packages link from
   `springboard` (`shell/books`, `clock`, `music`, `weather`, `calendar`, `files` lib, `deildu::scroll`) moves into a
   shared crate so packages stop linking the shell.
3. **System channel.** A privileged request and reply pair for `kind = "system"` packages, served by the shell or by a
   system daemon under `bifrostd`. It covers prefs, `Info`, the `Effect` set, the dock model, appearance, default apps
   and login items. Then Settings (`docs/settings-system-app.md`), the App Store UI (installs stay privileged, through
   `updated`) and Terminal (its PTY sessions must leave the shell) move out.
4. **Chrome as layer-shell clients.** The prerequisites have shipped: `bifrost-wm` serves `ext-background-effect-v1`
   and `bifrost_material_v1` (glass for any client), foreign-toplevel and screencopy. The `springboard/chrome` crate
   (`menubar`, `dock`, `panels` clients, link `bifrost-chrome.sock`, shell side `src/shell/chrome_host/`) exists and starts
   with `BIFROST_CHROME=1`; it is not the default yet (see `docs/desktop-chrome.md`). Order: volume HUD, banners, screenshot and Markup, Search, Notification Center,
   Today and widgets, Control Center, then dock, switcher and Overview, then the menu bar.
5. **Stays in the shell:** the lock screen and passcode, and the open and save panel with the doc broker. These are
   security boundaries.

Each component that leaves the shell updates through the App Store or OTA path and relaunches on its own.
