# Wayland protocol filter for native apps (design, S5)

Status: design only. No code changes. Implementation is a later task.

## Problem

Native (non-SDK) packages receive the real compositor socket. Their sandbox
binds it into the child namespace and exports `WAYLAND_DISPLAY`, so any
protocol the compositor advertises is reachable from the app. The compositor
is labwc (wlroots), per `docs/wayland-desktop.md`. Under wlroots that set
includes screen capture, input injection, overlay surfaces, and window
enumeration. `docs/app-isolation.md` (lines 32-33) lists privileged-protocol
filtering as future work; this document is that plan.

## Current state (verified against code)

- `springboard/src/bare/app_policy.rs:263-281`: when `native` is true, the
  sandbox validates `WAYLAND_DISPLAY` (no `/`), binds
  `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` with `--bind` at line 275, binds
  `session.json` read-only (line 271-274), binds `/dev/dri` with `--dev-bind-try`,
  and sets `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `WINIT_UNIX_BACKEND=wayland`
  (lines 280-281). The socket is passed through unchanged.
- Who passes `native = true`: the Browser launch (`springboard/src/shell/desk.rs:398`)
  and generic native packages (`desk.rs:685`). SDK apps go through
  `springboard/src/shell/apphost.rs:215` with `native = false`. They get no
  Wayland socket and `--disable-userns`, so they are not affected by this design.
- `/tmp` is a private tmpfs (`app_policy.rs:217-218`). `/run` is a directory
  skeleton, so the host runtime directory is not mounted.
- Capability `network` keeps the host network namespace (`app_policy.rs:188-189`,
  `--share-net`). Abstract UNIX sockets live in the network namespace,
  not the mount namespace, so tmpfs isolation does not hide them. See residual
  risks.
- The trusted shell (`springboard/src/bare/wayland.rs:7-15`) binds
  `zwlr_layer_shell_v1`, `zwlr_foreign_toplevel_manager_v1`,
  `ext_session_lock_manager_v1`, `zwlr_output_power_manager_v1`,
  `ext_idle_notifier_v1`, `xdg_activation_v1`, and `wp_cursor_shape_manager_v1`.
  The shell must keep the raw socket; the filter applies only to sandboxed children.
- WayVNC for Remote does not go through `app_policy::command`. It runs as the
  `bifrost-remote` systemd unit (`app_policy.rs:306-322`) and connects with the
  session environment (`springboard/apps/remote/assets/server.py:71`). It needs
  screencopy on the real socket and must remain unfiltered.
- `session.json` is written by `packaging/arch/wayland/session` (line 39-43
  waits for it). Its exact contents are not verified here; it must be
  audited for socket paths or secrets before it stays visible to children.

## Threat model

Goal: a sandboxed native child that is compromised or malicious must not be
able to observe or control other windows, the desktop, or the session, using
Wayland. Not in scope here: kernel exploits, the compositor's own bugs, and
the SDK path (already has no socket).

| Abuse | Protocol(s) | Effect |
|---|---|---|
| Capture other windows/screen | `zwlr_screencopy_manager_v1`, `ext_image_copy_capture_manager_v1`, `ext_output_image_capture_source_manager_v1`, `ext_foreign_toplevel_image_capture_source_manager_v1`, `zwlr_export_dmabuf_manager_v1` | Read pixels of other apps and the lock screen |
| Inject keys/pointer | `zwp_virtual_keyboard_manager_v1`, `zwlr_virtual_pointer_manager_v1` (and the ext variant if present) | Synthesize input into focused windows |
| Read keystrokes globally | `zwp_input_method_manager_v2`, `zwlr_input_inhibit_manager_v1` (inverse: blocks others) | Grab all input or block the lock screen |
| Overlay/spoof UI | `zwlr_layer_shell_v1` | Place surfaces above everything, fake lock/system UI |
| Enumerate windows | `zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1` | Titles, app IDs, activity |
| Clipboard snooping | `zwlr_data_control_manager_v1`, `ext_data_control_manager_v1` | Read/write clipboard silently (bypasses focus) |
| Display/power control | `zwlr_output_manager_v1`, `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`, `wp_drm_lease_device_v1` | Change modes, power off, tamper with colour, take DRM output |
| Session control | `ext_session_lock_manager_v1` | Lock or deny the session |
| Cross-app surfaces | `zxdg_exporter_v2`, `zxdg_importer_v2` | Embed another app's surfaces |
| Legacy auth bypass | `wl_drm` | Pre-dmabuf auth path; not needed with `zwp_linux_dmabuf_v1` |
| Focus theft | `xdg_activation_v1` | Raise own window over others; allowed only with a valid recent seat serial |
| Sandbox escape helper | `wp_security_context_v1` | Only the compositor may mark a connection sandboxed; the app must never create one |

Deny-by-default: the allowlist below is the only set of globals the child may
see.

## Approach: client-side proxy

Chosen: a per-launch proxy process that owns a new socket and speaks Wayland
to both sides.

- Proxy listens on `$XDG_RUNTIME_DIR/<name>` where `<name>` is a fresh socket
  (for example `wayland-app-<random>`) in a directory the shell creates for
  the launch. The child receives `WAYLAND_DISPLAY=<name>`. The raw
  `wayland-0` is never bound into the child.
- Proxy connects upstream to the real compositor socket using the shell's
  session environment. It runs outside the child's bubblewrap, under the same
  account, so the child cannot reach the upstream path.
- Proxy handles two things:
  1. Registry filtering (main control). It rewrites `wl_registry.global`
     events so only allowlisted interfaces are advertised. Interfaces not
     advertised are never exposed to the child. On `wl_registry.bind`, the
     proxy checks that the global name it received matches an advertised
     allowlisted interface and version. A name that was never advertised, or a
     version above the allowed maximum, gets a protocol error and the connection
     is closed. Clients can guess numeric global names, so the bind check is
     required, not optional.
  2. Object-level rules for allowlisted globals that can reach privileged
     behaviour: `xdg_activation_v1` tokens need a recent input serial from the
     same seat; `wl_data_device` and primary selection are allowed for focus-
     based clipboard only (no data-control).
- File descriptors (`SCM_RIGHTS`) for shm, dmabuf and keymap pass through
  unchanged. The proxy must not drop or reorder fd-carrying messages.
- Lifetime: one proxy per native child. The proxy exits when the child closes
  its connection or when the child exits. If the proxy cannot start or
  connect, the launch fails (fail-closed, matching `app_policy.rs`). No
  fallback to the raw socket.
- Implementation options (decide in the implementation task): a small Rust
  binary built on the `wayland-server` and `wayland-backend` crates, with
  `wayland-client` for upstream. The shell already depends on `wayland-client`
  0.31 (`springboard/Cargo.toml:24`). Pin versions when implementing.
- Preferred future option: if labwc implements `wp_security_context_v1`,
  the shell can ask the compositor to restrict the child's connection instead
  of proxying. Verify labwc support first; the proxy remains the fallback.

The shell (trusted) keeps the raw socket and is not proxied.

## Allowlist (initial)

Globals a native app needs. Versions are the maxima the proxy advertises.

| Global | Purpose |
|---|---|
| `wl_compositor`, `wl_subcompositor` | surfaces |
| `wl_shm` | software buffers |
| `zwp_linux_dmabuf_v1` | GPU buffers (Browser, GL apps) |
| `wl_seat` | input, focus-scoped by the compositor |
| `wl_output` | monitor info |
| `xdg_wm_base` | toplevels |
| `xdg_activation_v1` | restricted per object rules above |
| `xdg_decoration` (`zxdg_decoration_manager_v1`) | window decorations |
| `wp_viewporter`, `wp_fractional_scale_manager_v1` | scaling |
| `wp_presentation` | frame timing |
| `wp_cursor_shape_manager_v1` | cursors |
| `wp_content_type_manager_v1`, `wp_alpha_modifier_v1`, `wp_single_pixel_buffer_manager_v1` | rendering hints |
| `wl_data_device_manager`, `zwp_primary_selection_device_manager_v1` | focus-scoped clipboard |
| `zwp_text_input_manager_v3` | text input for the app's own fields |
| `zwp_idle_inhibit_manager_v1` | keep screen on while playing |
| `zwp_keyboard_shortcuts_inhibit_manager_v1` | app-owned shortcuts |

Review before allowing: `ext_idle_notifier_v1` (idle time leak; denied by
default for children). Denied always: everything in the threat table.

Blocked-by-default for every child, including future manifests: `wl_drm`,
`zwp_virtual_keyboard_manager_v1`, `zwlr_virtual_pointer_manager_v1`,
`zwp_input_method_manager_v2`, `zwlr_input_inhibit_manager_v1`,
`zwlr_layer_shell_v1`, `zwlr_foreign_toplevel_manager_v1`,
`ext_foreign_toplevel_list_v1`, `zwlr_screencopy_manager_v1`,
`ext_image_copy_capture_manager_v1`, `zwlr_export_dmabuf_manager_v1`,
`zwlr_data_control_manager_v1`, `ext_data_control_manager_v1`,
`zwlr_output_manager_v1`, `zwlr_output_power_manager_v1`,
`zwlr_gamma_control_manager_v1`, `wp_drm_lease_device_v1`,
`ext_session_lock_manager_v1`, `zxdg_exporter_v2`, `zxdg_importer_v2`,
`wp_security_context_v1`.

Manifest escape hatch: none in the first rollout. If a future app needs a
blocked global, it needs an explicit new capability with its own review, and
it may not be granted to Browser or the generic native path without a
recorded reason.

## Rollout plan

1. Inventory (no behaviour change). On Midget, record the globals labwc
   advertises (read-only `wayland-info`-style probe from the shell's session).
   Save output under `out/wayland-filter/`. Confirm `wp_security_context_v1`
   and XWayland status.
2. Audit mode. Build the proxy with a log-only switch: it forwards everything
   and logs every bind. Run Browser and one more native package in the PC VM
   to get the real required set. Update the allowlist from the logs.
3. Enforce for Browser only. Proxy-enforced launch with fail-closed startup.
   Browser must render, type, paste, resize, close, and open URLs. Rollback
   switch: a shell-config flag that disables enforcement; a release with the
   flag off is still fail-closed for the launch path itself.
4. Enforce for all native packages (`desk.rs:685` path). Keep the flag until
   one full Final QA run is green.
5. Remove the flag. Update `docs/app-isolation.md` lines 32-33 to say the
   filter is in place, with the list of blocked protocols and residual risks.
   Publish through `bifrost pc publish` with a CHANGELOG entry, per AGENTS.md.

Each step is one commit on its own branch and one board item, so a regression
can be backed out without losing the earlier steps.

## Test plan

Host (no device needed; run in the implementation task, not here):

- Unit: registry rewrite advertises exactly the allowlist; bind to an
  unadvertised numeric name is rejected; bind with a version above the maximum
  is rejected; fd-carrying messages pass through unchanged.
- Integration: an in-process fake compositor (wayland-server) exposing every
  blocked global. Client probes must see none of them, and the proxy must
  close the connection on forged binds.
- xdg_activation: token request without a serial is refused; with a stale
  serial is refused.
- Launch: proxy fails to start, then launch returns an error and no child is
  spawned.

PC VM and Midget (live, per `docs/bifrost-pc-workflow.md`):

- Negative probes from inside the sandbox, using the child's environment:
  screencopy capture (for example `grim`) fails; virtual keyboard or pointer
  injection fails; foreign-toplevel listing is empty; layer-shell surfaces are
  rejected; output configuration and gamma change fail; data-control clipboard
  fails.
- Positive probes: Browser window, typing, focus-scoped copy/paste, resize,
  close, and reopen; the same Browser session must not be able to read the
  Remote app's stream.
- Remote regression: Start/Stop Sharing still works and WayVNC still captures
  the desktop (it uses the raw socket, not the proxy).
- Shell regression: lock, unlock, layer-shell chrome, foreign toplevel
  taskbar, and output power still work.
- Record captures and logs under `out/wayland-filter/`.

Keep Browser and the native-package tests in the desktop path.

## Residual risks

- XWayland: an X11 client under the `network` capability shares the host
  network namespace, so an abstract socket such as `@/tmp/.X11-unix/X0` may be
  reachable even though `/tmp` is a tmpfs. Verify on Midget. If reachable,
  either deny `network`-capable native apps DISPLAY access or block the abstract
  namespace (for example with a separate network namespace plus a proxied
  socket). X11 bypasses the Wayland filter entirely.
- DRM render nodes (`/dev/dri`) are bound for GPU use. A render node cannot
  scan out or capture other clients' buffers, but it is still a large kernel
  attack surface. Out of scope here.
- Browser's nested sandbox has its own process tree; it must also use the
  proxy, not the raw socket. Verify in the first enforce step.
- `session.json` is bound read-only into every native child. Audit its
  fields before the rollout; remove anything that is a secret or a socket path
  the child does not need.
- The proxy is a new privileged-adjacent component parsing untrusted protocol
  input. It needs its own review, fuzz/negative tests, and a crash policy
  (child fails closed, never falls back to raw).
- Protocol names come from wlroots and ext-* staging sets; labwc's
  advertised set must be checked against them during the inventory step.
  This document does not claim labwc advertises any specific global.

## Open questions

- Does labwc implement `wp_security_context_v1`? (Decides proxy vs compositor
  restriction.)
- Does the Browser need `zwlr_layer_shell_v1` or `zwp_text_input_manager_v3`
  for its own UI, or can it use xdg-shell only?
- Should `ext_idle_notifier_v1` be available to native apps (video playback
  inhibit is covered by `zwp_idle_inhibit_manager_v1`)?
