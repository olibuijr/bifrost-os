# BifrOSt apps: processes, protocol, packages, store

Status: spec for the app split (2026-10-05). SpringBoard (the shell) stays the compositor and
system UI; every app becomes its own process, installed and updated from the store at
https://bifrost.olibuijr.com.

## Process model
- App = one executable built for x86_64-linux with the `bifrost-app` SDK crate
  (`springboard/sdk`; `bifrost_app::kit` re-exports the shell's Surface, Ui text, list and nav-bar kit). The shell spawns it with `BIFROST_SOCK=<fd number>` (an inherited
  `socketpair(AF_UNIX, SOCK_SEQPACKET)` end) and `BIFROST_DATA=<root>/appdata/<id>` (also the working directory) and
  `BIFROST_SHELL_DIR=<root>` (SFUI.ttf and the symbol fonts the UI kit loads).
- The shell keeps an app alive while it is in the switcher; it may kill a background app at
  any time (memory). Apps persist state themselves on `Suspend`.
- A crashing app never takes down the shell: the shell shows the app's last frame, then its
  icon/launch screen again on next open, and logs `app <id> exited <status>`.

## Frames (shared memory)
- The app allocates two buffers with `memfd_create`, each
  `w * h * 4` bytes, premultiplied RGBA8 packed as the shell's `u32` pixels (see `gfx::pack`),
  stride = w. It sends both fds once with `SCM_RIGHTS` in `Buffers`.
- Per frame: the app draws into the buffer the shell is not holding, sends `Frame { buf, damage }`,
  and may not touch that buffer until the shell returns it with `Release { buf }`. The shell keeps
  the newest frame's buffer held (switcher card, crash screen) and releases the older one when a
  newer frame arrives. `damage` is relative to the previous frame; the SDK keeps both buffers
  consistent (it copies the other buffer's recent damage before drawing), and the first frame after
  `Buffers` is always full.
- Size: the shell sends `Configure { w, h, scale, safe_top, safe_bottom, dark, text_scale,
  reduce_motion }` before the first frame and on every rotation/resize; the app reallocates
  and resends `Buffers`.

## Messages (SOCK_SEQPACKET, one message per packet, little-endian, `u32 tag` first)
Shell -> app: `Configure`, `Touch { kind: down|move|up|cancel, id, x, y, t_ms }`,
`Key { text | keycode }`, `Release { buf }`, `Resume`, `Suspend`, `Quit`, `Open { url }`.
App -> shell: `Hello { sdk_version, app_id }`, `Buffers { w, h }` (+2 fds), `Frame { buf, damage: [x,y,w,h]* }`,
`WantsKeyboard { on }`, `Haptic`, `OpenUrl { url }`, `Notify { title, body }`, `Log { text }`.
Clipboard (added with the SDK): shell -> app `Clipboard { text }` (on change), app -> shell `SetClipboard { text }`.
Exact encoding lives in `springboard/proto/src/lib.rs` (crate `bifrost-proto`, no dependencies, shared
by the shell and re-exported by the SDK as `bifrost_app::proto`; it also holds the socket/SCM_RIGHTS/
memfd FFI; round-trip unit tests). Tags: shell->app 1..9 in the order listed above, app->shell
0x101..0x109. Strings are `u32 len` + UTF-8. `Key` is `u8 kind` (0 text, 1 keycode) + string or `u32`. Unknown tags are ignored (forward compatible); `sdk_version`
bumps on breaking change.

## Control endpoint (PC, 2026-10-08)
Apps on the PC also serve JSON-RPC 2.0 on `$XDG_RUNTIME_DIR/bifrost/apps/<id>.sock` (see
`docs/bifrost-pc-workflow.md`, "App control endpoint"). It lives in `sdk/src/control.rs`,
and `sdk/src/runtime.rs` polls it with the shell channel (`Channel::wait_with`, `proto/src/sys.rs`).
The app implements `App::rpc_methods` and `App::rpc(ctx, data_dir, method, params)`; the SDK
answers `app.info` and `app.methods` itself. Handlers run on the app's main thread. The SDK change
is shared; every build binds a socket (`control.sock` in the data directory).

## Package (`.aap` = BifrOSt app package)
An uncompressed POSIX tar with:
- `manifest.toml`: `id` (reverse-DNS, e.g. `is.olibuijr.calculator`), `name`, `version` (semver),
  `exec` (path inside the package), `icon` (path to a 180x180 PNG), `min_shell` (semver),
  `summary`, `category`, `permissions = ["network", "audio", "camera", "storage", "photos", "notifications"]`.
- the executable, the icon, optional `assets/`.
Install: `/data/local/springboard/apps/<id>/<version>/` + `current` symlink; old version kept
until the new one has launched once (rollback). Built-in system apps (Settings, Terminal,
Control Center...) stay inside the shell.

## Store API (served by bifrost-site)
- `GET /api/v1/apps.json` -> `{ "apps": [ { id, name, version, summary, category, size, sha256,
  icon: "/store/<id>/icon.png", package: "/store/<id>/<id>-<version>.aap", min_shell } ] }`
- `GET /store/...` package and icon files.
- `GET /ota/springboard/latest.json` (+ binaries): shell updates (replaces titan :8190).
- `POST /diag/<name>`: LAN only (not proxied publicly).
The shell verifies `sha256` of every download before installing (TLS via rustls).

## Shell side (implemented)
- `shell/apphost.rs`: `register(root)` scans `<root>/apps/<id>/current/manifest.toml` once at start-up
  (a newly installed app appears after a shell restart); installed apps follow the built-ins in
  `shell::apps()` as `AppId::External(slot)`. `AppHost` spawns, pumps, blits (damage only) and kills.
- A foreground change sends `Suspend`/`Resume`; closing a card from the switcher sends `Quit` and
  then kills the process.

## Package kinds and OTA
Everything updates over the air by pull from the machine: the shell through Software Update
(`bifrost pc publish`), packages through the App Store (`publish-app.sh`). Experiment builds go through the fast-profile Midget push.

The kinds, in one table (the shell reads them in `src/shell/kinds.rs`, `tools/aap.py` checks
the directory, `bifrostd` accepts only `service`):

| kind | source dir | home grid | preinstalled | removable | exec | provides |
|---|---|---|---|---|---|---|
| `app` (default) | `apps/` | yes | no | yes | required | no |
| `system-app` | `apps/` | yes | yes | no | required | no |
| `system` | `system/` | no | yes | no | optional | yes |
| `service` | `system/` | no | yes | no | required | no |

An unknown `kind` is rejected by the packager and the shell; `""` means `app`.
- `kind = "app"` (default): user apps (Calculator...). Removable. Updated from the App Store
  without a shell restart: the new version is installed beside the old, the running process
  (if any) gets `Quit`, the next launch uses `current`.
- `kind = "system-app"`: the stock apps moved out of the shell (Notes, Calendar, Music...).
  Not removable, preinstalled, updated by the store like apps (Updates tab, badge on the
  App Store icon, automatic by default).
- `kind = "system"` (implemented): data-only or shell-facing packages with no UI of their own,
  e.g. `is.olibuijr.keyboard` (`provides = ["keyboard"]`: emoji font in `assets/`, word lists in
  `assets/words/<lang>.txt`). `exec` is optional (only here). Without an `exec` a system package is
  never spawned. With one, and a `provides`, the shell spawns it on demand as the provider: the keyboard
  is `bifrost-keyboard` (crate `apps/keyboard`, 1.1.0), a standalone process that draws its own frame and sends what is
  typed back over the app socket (`docs/keyboard-system-app.md`). Never on the home screen. They show in the App Store (Apps > System, Updates; product page "System Component";
  no GET/OPEN), are installed when missing and updated silently by the background check (no
  banner, always on; badge counts them until installed), and the shell hot-reloads what they
  provide with no restart. The store's apps.json entry carries `"kind":"system"`. Sources live in
  `system/<name>/`; `tools/aap.py pack system/keyboard` builds them (see its README).
- `kind = "service"`: background daemons without UI (audio, network, update agent, sensors).
  Supervised by `bifrostd`, a small supervisor started by `boot.sh` before the shell; it
  keeps them running (restart with backoff, never gives up), speaks to the shell over
  `/data/local/springboard/run/<id>.sock`, and swaps a service to a new version by starting
  the new one, health-checking it, then stopping the old one (rollback on failure). The
  shell reconnects to a service socket transparently, so a service update never restarts it.
- The OS itself (shell + `bifrostd` + `boot.sh`) updates through Settings > General >
  Software Update: download, verify, stage `springboard.new`, "Install Now" restarts the
  shell into `boot.sh` probation with automatic rollback. Availability shows as an iOS
  notification and red badges (Settings icon, General, Software Update).
- Update checks: at start-up, on Wi-Fi connect and every 15 minutes; downloads automatic, installs
  of the OS always ask; system packages always install in the background; App Store apps update in the background while closed (Settings › Software Update › Automatic Updates, on by default).

## Scrolling
Every scrolling list uses `bifrost_app::kit::Scroller` (the shell's own lists use the same type), so
drag, fling, deceleration and edge bounce feel the same everywhere. Pass touch times from `Input::Touch`
(they are the screen's sample times), call `tick(t)` every frame while `animating()`, and draw at
`offset()`. Do not write custom scroll physics.

### Desktop context menus (shell 0.4.28)

The SDK delivers `Input::Context { x, y }` for a real secondary mouse click,
with coordinates in app pixels. Call `ctx.context_menu(x, y, items)` to request
a shell material menu. Each `ContextItem` has `id: u32`, `label: String` and
`enabled: bool`; selection returns `Input::ContextAction(id)`. Disabled rows do
not dispatch actions. Menus accept at most 32 items and bounded labels, require
the requesting app to be foreground, and are clamped to the desktop bounds.
The wire extension is additive: older clients skip unknown message tags.
Files 1.0.5 demonstrates Open, Quick Look, Copy Name and Refresh.

Desktop links sent with `ctx.open_url()` now open BifrOSt Browser through the
central app policy. HTTP/HTTPS also use the default `bifrost-browser.desktop`
association. `springboard --install-store-app ID` installs a PC catalogue app
using the same download/hash/archive verification as App Store, for setup and
administration. It requires root and does not embed app packages in OS updates.
