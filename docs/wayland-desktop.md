# BifrOSt PC Wayland port

The EFI loader frame and Bootweave run before a Wayland server exists (no Plymouth). The retained
frame hands off to `bifrost-wm` after userspace starts; `docs/boot-architecture.md` has the boot side.
The live installer and installed desktop share `/usr/lib/bifrost/session`.

## Runtime

`springboard.service` runs the session supervisor. It starts `bifrost-wm`, the only window server, as the
installed local account with seatd for DRM/input access. BifrOSt connects with
its `desktop-wayland` feature; it never grabs evdev or becomes DRM master in
this mode. The installer follows the same path using a live-only account.
System operations remain in the existing privileged installer/shell process;
external graphical apps run as the local account.

The desktop is a wlr-layer-shell background. Every built-in app has an
xdg-shell toplevel, its existing BifrOSt titlebar and layout, and its own released
SHM buffers. Native Linux apps share the compositor, input focus and window
stack. The compositor supports XWayland for X11 clients. Applications is the
last dock button. The compositor owns the key chords (`docs/window-server.md`, "Key bindings"): Super+Space opens
the search panel, Ctrl+Up and F3 open Mission Control (Super+A is not bound: Cmd+A is Select All). `bifrost-apps firefox` opens Firefox. The app launcher forwards only the session
variables it needs and drops root privileges before invoking the application.

BifrOSt locks through `ext-session-lock-v1`, which blocks external app input and
keeps the session locked if the lock client dies. Screenshot tools use compositor
capture (`wlr-screencopy` and `ext-image-copy-capture`, GLES readback in `wm/src/capture.rs`) rather than the old shell
framebuffer, so Linux windows are included. If `bifrost-wm` exits three times within 60 s the session restarts it in
safe mode (GLES, no direct apps, no chrome clients); three more exits hand over to the recovery flow.

## Performance boundaries

External apps use the compositor's native GPU buffer path, including dma-buf
where the client/driver supports it. BifrOSt's existing raster renderer submits
SHM buffers: background damage is tracked across both buffers, unchanged app
frames are skipped, and pixel conversion uses packed channel operations.
This is not a zero-copy GPU port of the BifrOSt raster renderer. VM software
rendering measures functional behavior, not physical Intel GPU throughput.

## Build and delivery

Build PC shell and installer with `--profile fast --features desktop-wayland`.
`bifrost pc publish` includes the feature, packages the compositor/session
requirements and syncs the 8079 package mirror (`tools/pc_mirror.py`: core and extra plus every package a guarded `pacman -Su` needs, fetched from the Arch
upstream, then checked). The normal update remains pacman plus System Update. The ISO
build includes the same session files and dependencies; ISO rebuilding is
necessary here because the live boot and installer entry points changed.

The installer preserves XKB layout/variant, copies session support and registers
its exact shell files with pacman. Its initial version is below the matching
published timestamped package, so the first update can replace it normally.

## Acceptance before physical installation

Verify in the PC VM: firmware/Plymouth → Wayland installer, account/layout/disk
pages, installation, installed UEFI boot, unlock/lock/relock, Settings and its
Displays page, native window move/resize/close/minimize/restore, typing, app
launch/menu, Firefox plus an X11 app, Linux/native focus switching, screenshot,
Software Update check/install, compositor failure/recovery, and a reboot.
Save captures and logs under `out/wayland-vm/`. Publish/install on Midget only
after this record passes. Midget additionally verifies native refresh and GPU.

Protocol/runtime references: [Smithay](https://github.com/Smithay/smithay),
[wayland-client](https://docs.rs/wayland-client/latest/wayland_client/).

### Window memory ownership (0.4.13)

Immutable window images are shared between the desktop cache and backend. Each
Wayland window begins with one SHM buffer; it grows to two only if a damaged
frame arrives before the compositor releases the first. Two busy slots defer
submission until release. Every new slot starts fully damaged, and existing
slots accumulate changes independently. Never overwrite an attached busy slot.
Buffer reuse requires matching width and height, not merely matching area;
equal-area reshapes still change the Wayland stride and buffer metadata.

### Desktop menus and notifications (0.4.15)

The menubar has a dedicated Top layer with an exclusive zone; dropdowns use
Overlay layer surfaces with independent bounded SHM buffers. Pointer events
translate to desktop coordinates; opening menus preserves app keyboard focus.
NetworkManager and power-profile D-Bus notifications trigger worker snapshots
and a pollable UI notification socket, with a 60-second recovery fallback.
Power profiles require power-profiles-daemon, enabled by packages and installer.
System battery discovery excludes power supplies with scope=Device.

Partial SDK damage updates retained rounded window composition. Diagnostic
performance updates can repaint just their background band without rebuilding
quiet app frames. Unchanged foreign-window lists do not invalidate the desktop.
Copy-on-write and SHM transfers remain; this is not a fully GPU raster renderer.

### GPU presenter (0.4.56)

The desktop background, lock surface, menu bar, dock and menus present GBM dma-bufs through
linux-dmabuf-v1, rendered by `gpu::wl::Presenter` (wallpaper, GPU glass, content layer). SHM remains
for built-in app windows and as the fallback (`BIFROST_GPU=0`). Details: `docs/gpu-first.md`.

Live blur and materials for all clients are composed by the window server, not the presenter (the presenter's live
backdrop was removed in sprint 6); see `docs/gpu-first.md` and `docs/window-server.md`.
