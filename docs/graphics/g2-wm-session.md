# G2 wm-session: session protocols of the window server

Wave G2, row 5 (`wm-session`) of `docs/graphics-waves.md`. Owner: `springboard/wm/src/session_protocols.rs`. Contract:
`docs/graphics-architecture.md` ("Session parity with labwc"). This note is the record of what landed, what is open, and
how to check it on Midget.

## What is implemented

All the session protocols labwc serves today, served by `bifrost-wm` through `springboard/wm/src/session_protocols.rs`.
Smithay 0.7 has cursor shapes and xdg-activation (and foreign-toplevel-list, which this does not use). The wlr protocols
are written against the `wayland-protocols-wlr` server bindings that Smithay already enables (reached as
`smithay::reexports::wayland_protocols_wlr`), so no Cargo change was needed.

| Protocol | Version | State |
|---|---|---|
| `zwlr_foreign_toplevel_manager_v1` + handle | 3 | Full list (title, app id, activated, maximized, minimized), `done`, `closed`. Requests: `activate` (restores a minimized window and focuses it), `close`, `set_minimized`/`unset_minimized`, `set_maximized`/`unset_maximized` (through the same path as xdg-shell). `set_rectangle`, fullscreen requests: accepted and ignored (no model yet). |
| `zwlr_screencopy_manager_v1` + frame | 3 | `capture_output` and `capture_output_region`; the frame announces an XRGB8888 buffer at the capture size (`buffer_done`); `copy` checks the shm buffer and calls the backend hook. Without a backend the frame fails at once. |
| `ext_image_copy_capture_manager_v1` + sessions and frames | 1 | Output sources (`ext_output_image_capture_source_manager_v1`), a session that sends `buffer_size`, `shm_format`, `done`; frames: `attach_buffer`, `capture` (transform, damage, presentation time, ready). Cursor sessions are created and stopped. |
| `wp_cursor_shape_manager_v1` | 2 | Smithay's manager. The seat's requested image is stored in `SessionState::cursor`; the cursor is not drawn yet. |
| `xdg_activation_v1` | 1 | Smithay's state. A token with a serial younger than 10 s raises and focuses its window; the token is then removed. |
| `zwlr_output_manager_v1` (heads, modes, configurations) | 4 | Heads with name, description, physical size, make and model, modes (size, refresh, preferred), enabled, current mode, position, transform, scale, `done` with a serial. `create_configuration`: `enable_head`, `disable_head`, `set_mode`, `set_custom_mode`, `test`, `apply` (`succeeded`/`failed`). Heads are published on bind and after an apply. |
| `zwlr_output_power_manager_v1` + `zwlr_output_power_v1` | 1 | `get_output_power`, `set_mode` (on/off) through the power hook; `mode` and `failed` events. |

Pure logic (tested by hand, see below): `diff_toplevels` (added, changed, removed), `ToplevelInfo::state_bytes` (protocol
state enum order), `choose_mode` (preferred, exact size with nearest refresh within 1 Hz, fastest of a size),
`clamp_region`, `shm_stride` and `buffer_fits`, `activation_allowed`, `stamp` (timestamp split).

## How it is wired

- `springboard/wm/src/lib.rs`: `pub mod session_protocols;` (the one module declaration).
- `springboard/wm/src/state.rs`: `WmState::session: SessionState`, created in `WmState::new` (this registers the globals);
  `focus()` ends with `sync_toplevels()`.
- `springboard/wm/src/handlers.rs`: a window commit calls `sync_toplevels()` (titles change there); `set_maximized` is
  `pub(crate)` and calls `sync_toplevels()`; `cursor_image` stores the seat's image in `session.cursor`.
- Smithay's `delegate_xdg_activation!` and `delegate_cursor_shape!` are invoked in `session_protocols.rs`.
- Backend hooks, all in `SessionState::hooks` (`SessionHooks`): `capture` (pixels of a region as XRGB8888 rows),
  `set_mode` (DRM modeset, returns whether it took), `set_power` (DPMS). Nothing sets them yet. A request that needs a
  missing hook is refused (`failed`), never faked.

## Known gap: frames are not written into client buffers

`write_shm` in `session_protocols.rs` is a stub that returns an error. Copying a captured frame into a client's wl_shm
buffer needs a raw write into the pool, which is `unsafe` (`with_buffer_contents_mut` hands out a raw pointer). This
crate keeps `unsafe` out of its code, so every `copy` and `capture` is answered `failed` (after the buffer has been
checked and the capture hook has run). Screenshots with grim therefore fail until this is decided (see todo_left).

## Verification

Host (what was run): `cd springboard && CARGO_TARGET_DIR=.../target-agents cargo check --offline -j 2 -p bifrost-wm --tests`
passes with no warnings from the wm files. The unit tests in `session_protocols.rs` (9 tests) compile; they were
not executed by this agent (`cargo test` is outside its allowed commands). Expected values were worked out by hand:
for example the diff test expects `Changed(1)`, `Added(3)`, `Removed(2)` in that order, and the mode test expects
`choose_mode(modes, (1920, 1080, Some(75000)))` to be `None` because the nearest rate (60 Hz) is 15 Hz away, beyond the
1 Hz tolerance.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4). The session (history: ran labwc before its removal) runs `bifrost-wm`; formerly unless `/etc/bifrost/compositor` says
`bifrost-wm`. To test the protocols:

1. Build and publish as usual (`bifrost pc publish`), then set `/etc/bifrost/compositor` to `bifrost-wm` and set
   `BIFROST_WM_RENDERER=vulkan` for the session. Reboot (`bifrost pc midget update --restart`), unlock with
   `bifrost pc midget key enter`.
2. Globals: `ssh midget 'wayland-info'` (or `wayland-info | grep -E "zwlr_|ext_image_copy|ext_output_image|wp_cursor|xdg_activation"`)
   must list `zwlr_foreign_toplevel_manager_v1` v3, `zwlr_screencopy_manager_v1` v3, `zwlr_output_manager_v1` v4,
   `zwlr_output_power_manager_v1` v1, `ext_image_copy_capture_manager_v1` v1, `ext_output_image_capture_source_manager_v1`
   v1, `wp_cursor_shape_manager_v1` v2 and `xdg_activation_v1`.
3. Toplevel list: open two apps (`bifrost pc midget open-app ID`), then `wlrctl toplevel list` (or a client that binds
   the foreign-toplevel manager) must list both with their titles and app ids; `wlrctl toplevel activate` and
   `wlrctl toplevel minimize` must move focus and minimize; the Dock and switcher follow.
4. Output: `wlr-randr` must list the panel with its modes; `wlr-randr --output <name> --mode 1920x1080@60` must answer
   ok when the mode is the current one (a different mode fails until the DRM modeset hook exists).
5. Power: `wlopm --off <name>` fails until the DPMS hook exists; with the hook it turns the panel off and on.
6. Screenshots: `grim /tmp/shot.png` fails with the stub (expected, see the gap above); the log shows `failed`.

## todo_left

- `write_shm`: decide the one allowed `unsafe` (or a safe shm write in `bifrost-gpu`), then screencopy and ext capture
  work end to end. Afterwards bless a screenshot test in the headless harness.
- Backends set the hooks: `capture` from the Vulkan output target (drm) and the headless texture; `set_mode` (atomic
  modeset) and `set_power` (DPMS) in `drm.rs`.
- Output configuration: position, transform, scale and adaptive sync are accepted but not applied; disabling a head is
  accepted but does not switch the output off. Heads are republished only on bind and after an apply (not on hotplug).
- Foreign toplevels: no `output_enter`/`output_leave`, no `parent`, no fullscreen state (the window manager has no
  fullscreen model yet), `set_rectangle` ignored.
- Cursor: the stored image is not drawn; the cursor owner (`wm-input` or the scene) reads `session.cursor`.
- Cursor sessions of ext-image-copy-capture are stopped at once; the foreign-toplevel image source is not advertised.
- Activation with no serial is ignored (no urgency hint).
- Host tests are compiled but not run by this agent; the lead should run `cargo test -p bifrost-wm session_protocols`.

## Contract changes

None to the existing skeleton (the skeleton had no wm-session functions). The wm crate gains the `SessionState` and the
hook fields above; the backends must set them (see todo_left).
