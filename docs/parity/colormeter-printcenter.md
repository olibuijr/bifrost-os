# Digital Color Meter and Print Center (round 75)

Slug: `colormeter-printcenter`. Matrix entries this slice closes (the lead records them):

| Matrix entry | File | BifrOSt path |
|---|---|---|
| Digital Color Meter | `docs/parity-matrix/os/apps/d-f.json` | `springboard/apps/colormeter-desktop` (v0.2.0), `springboard/src/shell/services/screen_sample.rs`, `springboard/sdk/src/screen_sample.rs` |
| Print Center | `docs/parity-matrix/os/apps/p-r.json` | `springboard/apps/printers-desktop` (v0.1.0), `springboard/ui/src/cups.rs` (job hold and release) |

## Reference evidence

- `~/Projects/macos-ref/extract/ref27/System/Applications/Utilities/Digital Color Meter.app/Contents/Info.plist`
  (CFBundleIdentifier `com.apple.DigitalColorMeter`, version 6.11): the bundle has an `en`/`Base.lproj` tree and a main nib.
  Its menus, windows and controls were not decoded with REA in this round (the nib decode is still to do), so the
  toolbar set below is the one the app shows, not a measured copy.
- `~/Projects/macos-ref/extract/ref27/System/Applications/Utilities/Print Center.app/Contents/Info.plist`
  (CFBundleIdentifier `com.apple.printcenter`, version 1.0): carries `UTImportedTypeDeclarations`. Behaviour (printer
  list, queue, pause, resume, cancel, hold, release, default) is from the scheduler's IPP operations (RFC 8011 and the
  CUPS extensions in `ui/src/ipp.rs`), not from the bundle.

## Digital Color Meter

API: the shell's `screen` service (new methods) and the pixel path in the SDK.

- Shell, `src/shell/services/screen_sample.rs`: `sample({x,y,w,h})` (existing), `display()` returns `{w,h}`, and
  `pointer()` returns `{x,y}` or null. Every method needs the Screen permission. The pointer is recorded by
  `bare/wayland.rs` on each motion (`pointer_moved`, window offset plus window-local position) and cleared on leave
  (`pointer_left`). Unit tests cover the replies and the recording.
- SDK, `sdk/src/screen_sample.rs`: `display(ctx)`, `pointer(ctx)`, `sample(ctx, ..)`; decoders `decode_size`,
  `decode_point` and `decode`. Tests cover each reply shape.
- App, `apps/colormeter-desktop/src/source.rs`: `ShellSource` replaces `NoCapture`. `wants` asks for the display size
  until it is known, the pointer, and a square of `cells + 4` pixels centred on the readout pixel, with at most one
  request of each kind in flight. `answer` takes the reply. `read` serves any rectangle inside the last square that
  came back. `main.rs` sends the requests from `tick` (a 50 ms refresh loop through `next_wake`) and routes replies
  through `service_reply`.
- Done: real pixels in the magnifier (aperture cells, zoom), the readout (hex, decimal, percent in sRGB, Display P3,
  Generic RGB), the lock, copy as text and copy as swatch. Tested: the source logic, the loupe and pointer geometry, the
  toolbar cycle and the readout.
- Needs Midget checking: (1) the pointer units. The Wayland surface coordinates are assumed to equal display pixels
  (scale 1). With a scaled panel the readout will be off by the scale factor. (2) The loupe is one round trip behind
  the pointer (about 50 ms plus a whole-frame draw per sample). (3) The screen sample draws the whole frame, so the
  refresh rate should be checked for cost on Midget. (4) The Screen permission prompt when the app asks for it.

## Print Center

API: `bifrost_ui::cups::Client` (IPP over `localhost:631`), reached from the app through `bifrost_app::controls::cups`.

- `ui/src/cups.rs`: `hold_job` (Hold-Job, 0x000C) and `release_job` (Release-Job, 0x000D), added to `ipp::op`.
  Tests check the operation code and the job id on the wire, and that a bad printer name sends nothing.
- App, `apps/printers-desktop`:
  - `model.rs`: the state (printers, selected printer and its queue, selected job, footer), the actions, the reply
    rules, the labels and the toolbar enablement rules. Pure, with tests.
  - `worker.rs`: a thread owns the CUPS client; requests and replies travel on channels, so the window never waits
    on the scheduler. A finished action lists again.
  - `window.rs`: layout (toolbar, printer list, queue, footer), taps, the 5 s listing timer and the 50 ms wake while a
    request is in flight. Tests cover hit testing, selection, the job buttons and the timers.
  - `draw.rs`: paints the window with the kit's toolbar, colours and text sizes, and the accessibility nodes for the
    toolbar.
- Done: printer list with the default flagged and the queue counts; pause and resume (the toolbar toggles on the
  printer state); set default; hold, release and cancel a job (enabled from the job state); refresh.
- Not done: adding or deleting a printer (the client has `add_driverless_printer` and `delete_printer`, the window does
  not expose them yet); printer detail beyond make, location and state message; keyboard navigation.
- Needs Midget checking: that the scheduler answers on this image; a stopped scheduler shows its error in the footer;
  the toolbar labels and the layout at the full-size window; the Dark and Light appearance; `bifrost pc smoke
  printers-desktop` for the preview.

## Follow-up for the lead

- `ui/src/print.rs` still lists printers with `lpstat` (`printers()`, `parse_lpstat`). The Print sheet in
  `src/shell/print_sheet.rs` (line 492) calls it. That is a second printer list next to `cups::Client`. Moving the Print
  sheet onto `cups::Client::printers()` and deleting `printers()` and `parse_lpstat` is the adopt-and-delete step. It
  touches a shell file, so it was left for a round that can build it.

## Shared-file edits to merge with care

- `springboard/src/bare/wayland.rs`: two lines in the pointer handler (`pointer_moved` on motion, `pointer_left` on leave).
- `springboard/ui/src/ipp.rs`: two constants in `op`.
- `springboard/ui/src/cups.rs`: `hold_job`, `release_job`, `job_op`, and two tests.
- `springboard/Cargo.toml`: `apps/printers-desktop` added to the workspace members.
- `springboard/Cargo.lock`: the `bifrost-printers` package entry (path dependencies only, no new crates).
- `springboard/sdk/src/screen_sample.rs` and `springboard/src/shell/services/screen_sample.rs`: additions, existing
  functions kept.
