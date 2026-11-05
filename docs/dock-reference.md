# Dock placement and interaction, 2026-10-23

## Reference and limits

The design authority is the local Golden Gate desktop reference,
`goldengate_refimages/27-Golden-Gate-Desktop.jpg`, together with the HIG checklist
CHR-03 and the macOS structure reference's Dock menus section. The reference image
shows a centred bottom Dock separated from the screen edge by approximately six
image pixels. Our four-point inset is a visual calibration to that image, not a
constant recovered from the reference executable or a numerical HIG requirement.

The public behavior reference is Apple's [Use the Dock](https://support.apple.com/guide/mac-help/open-apps-from-the-dock-mh35859/mac)
and [Desktop & Dock settings](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac):
edge placement, adjustable size, optional magnification, auto-hide, launch and
activation, running indicators, context menus and reorderable items. Existing
user pin order and appearance preferences are retained.

The local 27.0 reference archive (26A434) was inspected with the
`reverse-engineer-anything` skill. The extracted Dock identifies itself as
2571.0.6.402, built for platform 27.0. The original universal binary SHA-256 is
`b704affba65f732ffd6676c3bb22c94abc737ee73af8bdf29a5ef02650cde33e`.
REA could not select a host-compatible architecture from that ARM-only universal
file. Its first ARM64e slice was extracted to a derived file outside this repo,
preserving the original, at file offset 16384, length 4843648; its SHA-256 is
`8aacf1a8f961f10e0e67e1949b98c21f6f06a396e95cb5f443a75c2dcafe602e`.

Retained REA observations:

- `ev_b31d0d0c02bb5d3e3371dda184ce9b7e5715a1ca4f22a33173f4c57190c44c5b`:
  ARM64 native overview, 29,442 procedures and 44,461 strings.
- `ev_ff066a179a0f0f876f9d606ab73675ab459f0a7d943472dfeeab130eb6056f1a`:
  named procedures for magnification, touch magnification, auto-hide and its
  event queue. These establish a behavior inventory, not executed behavior.
- `ev_78c1bfbdf189bee53df0f51b261e9df7379448af6daedaeac2c1dc1959829f6a`:
  the auto-hide delay getter reads a mouse-handler field. It does not establish
  the default delay, so this round does not claim a recovered timing constant.

Evidence and the analysis snapshot remain outside the repo at
`~/Projects/macos-ref/analysis/dock-27/`. No reference executable, assets or
decompiled implementation are shipped. There is no live reference system in
this session with which to compare animation trajectories.

## Problems in our implementation

- The resting bottom rectangle was offset twice using the reserved work-area
  depth. Its edge gap grew to about half the plate height instead of staying a
  small inset as the Dock size changed.
- The exported Dock surface included extra width for magnification but only the
  resting plate's depth. Magnified icon tops, bounce, labels and shadow could be
  cropped. Input and context menus also used the resting plate or slots.
- The compositor inferred the edge and reservation from the entire surface.
  Adding transparent animation space would therefore reserve too much screen;
  its old exclusive zone also counted the anchored margin twice. A side surface
  could be mistaken for a bottom surface from its aspect ratio.
- Work areas always removed a bottom strip, including for a side or auto-hidden
  Dock. The row-fitting budget omitted the Downloads and Trash tiles.

## Changes in 0.10.23

The plate is anchored to its selected edge with the named screen inset. The full
row is fitted before placement, scaling padding and separators with the icons.
Magnification stays within the output and keeps the visible icon's context-menu
target. A stable padded surface contains icons, labels, bounce and shadow;
input excludes the unused outer surface and labels.

Chrome frames carry the Dock edge and resting reservation explicitly. Transparent
surface padding does not alter the work area, anchored margins count once, and a
non-reserving auto-hidden Dock ignores other exclusive zones. Side placement
accounts for the menu bar in the compositor's already reduced arrangement area.

Window geometry and the screens service now reserve the selected edge. Filled
windows follow changes to Dock size, position and auto-hide; other windows keep
their chosen sizes. This extends the one-display visible-frame behavior. The multi-display
model is described under "Several displays" below.

## Validation

The serial workspace run passed: **33,425 passed, 0 failed, 187 ignored**.
Command (in `springboard/`):

```sh
BIFROST_LEAD_BUILD=1 ../tools/lead-run.sh cargo test --offline --profile fast --workspace --features springboard/desktop-wayland,springboard/testkit -j4 --no-fail-fast -- --test-threads=1
```

New tests cover all three edge positions at 640×480, 1280×800 and 2560×1440,
three icon sizes, complete-row fit, live magnified context/input targeting,
painted surface containment, all-filled-window reflow and visible screen frames.
The surface test excludes the menu bar's top strip, where side-Dock shadows are
intentionally clipped; actual icons are separately checked below the bar.

`bifrost pc acceptance --quick` stops at the existing 206 name-guard findings;
the changed source introduces zero new findings. SDK coverage is 91.9% (floor
79%). Parity check/build pass; built coverage is 99% against the 80% gate.
Logs are in `out/dock-2026-10-23/`. Device verification follows publication.

## Device follow-up in 0.10.24

Midget accepted `0.10.23.1792778326-1` from `9882b467ea02` with the service active
and zero restarts. Bottom hover now displays the entire magnified Mail icon and
label (`05-bottom-hover.png`); Left placement and a newly maximized Settings
window reserve the correct edge (`10-work-area-left.png`).

Switching the Dock from that filled Settings window to Right exposed a further
bug: the Settings input path temporarily sizes the shell to its window body.
Applying a Dock change within that path subtracted the bar and Dock from the
already reduced window size. The screenshot and state show a window of
2368×1370 on the 2560×1440 output, leaving too much space.

The follow-up defers Settings' queued Dock changes until desktop event handling
returns from `with_size`. Reflow therefore uses the output size. A regression
enters the real Settings content-event path with a filled window and changes
Left, Right, Bottom and auto-hide. Auto-hide also keeps a hovered magnified icon
visible above the plate rather than scheduling a hide while still over the icon.
Follow-up host validation passed: **6,566 shell tests, 0 failed, 22 ignored**,
including both new regressions. Command (in `springboard/`):

```sh
BIFROST_LEAD_BUILD=1 ../tools/lead-run.sh cargo test --offline --profile fast -p springboard --lib --features desktop-wayland,testkit -j4 -- --test-threads=1
```

The full workspace already passed in this batch before the shell-only follow-up.
0.10.24 device verification is recorded below.

### 0.10.24 on Midget

Guard accepted `0.10.24.1792779718-1`; build-info is `ea92e2228609`, service active,
zero restarts. After explicitly maximizing Settings (session restoration keeps
its rectangle as a floating window), the real Settings content-event path passed:

| Dock position | Settings frame, compositor pixels |
| --- | --- |
| Left | 96, 35, 2464, 1405 |
| Right | 0, 35, 2464, 1405 |
| Bottom | 0, 35, 2560, 1309 |

Captures 18–20 and `08-actions.log` show no repeated subtraction. Midget checks
use the repository-prescribed window-local `tap-window` input path. Temporary
uinput pointer clicks were unreliable even in Settings; this limits attribution
of the Mail input findings. Hover is also checked through shell hover events.
Appearance and auto-hide results are recorded below.

Auto-hide was verified with the same filled Settings window: it expands to
`0,35,2560,1405`, hides after leaving, reveals at the edge, and stays visible for
more than the hide delay while hovering the enlarged App Store icon above the
plate. Leaving hides it again; disabling auto-hide returns to `2560×1309`.
Captures 21–24 and `autohide-verified.log` retain the evidence. The context menu
at the enlarged icon's top identifies App Store (capture 25).

Dock hover surfaces were inspected on Midget in Light (capture 26) and Dark
(capture 27). The original Dock preferences compare byte-for-byte equal after
restoration: Bottom, size 48, auto-hide off, magnification on, size 80 and the
same pin order. Dark was restored, Settings returned to a floating window, and
the temporary pointer device was removed. Final desktop: capture 28.

The external parity database connector is unavailable in this session. The
updated export is retained in `out/dock-2026-10-23/followup-parity-db/`; no database
publication is claimed. The row remains partial because multi-display support
is still incomplete. These checks do not establish full reference-OS parity.

## Several displays

The Dock, like the menu bar, lives on one display: the main one. Every other display has its whole frame as its
visible frame, with no Dock or menu bar reservation (the shell draws one menu bar, so separate per-display bars are not
modelled). `shell/displays.rs` holds the `Arrangement` (outputs in the shell's pixel space plus the main index) and
`Shell::set_outputs` takes it from the backend. Zoom (Fill), tiling, edge snapping, full screen, restoring a placed
window and the Dock-change reflow of filled windows use the visible frame of the display the window is on (the one
showing most of it); dragging a window may cross onto another display. The `screens` service and `bifrost_app::screens`
list every output with its own frame and visible frame, and the main flag marks the Dock's display. New windows still
open on the main display. One display (or none reported) behaves as before.

Tests: `shell::displays::tests` (two fake outputs: visible frames per Dock edge, display choice, zoom, tile, full
screen, restore, drag) and `services::screens::tests` (two-output list). Not done: the backend still binds the first
`wl_output` only, so nothing calls `set_outputs` on hardware yet; drawing the desktop on a second output needs a
surface per output. Midget has one display, so this is host-tested only.
