# BifrOSt boot architecture (contract)

Owner request 2026-10-12: a boot as smooth as the reference platform's, in the BifrOSt loader, Bootweave and BifrOSt.
Like `docs/graphics-architecture.md` and `docs/audio-architecture.md`, this is the contract each component is written
against; when code and this document disagree, fix one of them in the same change. The tasks are in `tasks/boot/`
(git-ignored, like `tasks/audio/`). The history and earlier decisions are in `docs/bifrost-boot-design.md`.

Reference names appear only in this document. Code, comments, strings and file names use neutral names
(`bifrost pc names`).

## How the reference platform does it

| Idea | What the reference platform does |
|---|---|
| One picture | Firmware or the boot loader draws the logo at the panel's native mode. The kernel's display driver and then the window server adopt that mode and that framebuffer's contents. No modeset, no blank frame, no brightness change. |
| No text | No firmware menu, no kernel console, no service log. Verbose mode and the boot picker are opt-in (a held key). |
| An honest progress bar | The bar advances against the duration of the previous boot, so it moves smoothly instead of in jumps. |
| Nothing discovered at boot | A prelinked kernel collection (kernel plus exactly this machine's drivers), shared library caches, and a sealed, signed system volume are prepared at install and update time. Boot loads, it does not probe. |
| Services on demand | launchd starts daemons in parallel and most only when first messaged. Nothing waits on a dependency chain. |
| Unlock is login | With full-disk encryption the password is typed before the OS loads, and the user is then logged straight in. |
| Reveal once | The window server keeps the boot image until the first complete desktop frame (wallpaper, menu bar, Dock) and then fades once. |
| Symmetric shutdown | Shutdown and restart return to the same image; no text. |

## Where BifrOSt is (2026-10-12)

- Loader: `tools/splash-efi` (Rust, `no_std`, UEFI). Paints a 160 px logo through GOP, reads `EFI/BIFROST/boot.cfg`,
  picks current or rollback (`boot_policy.rs`), starts the Linux EFI stub with `initrd=... quiet splash`. It sets the
  panel's largest GOP mode: the boot's only modeset.
- Kernel: inherits the mode (simpledrm, then the GPU driver with fastboot). Initramfs hooks
  `base udev autodetect microcode modconf kms keyboard block filesystems bifrost-mounts`, busybox init.
- Early userspace: `springboard.service` (no default dependencies) runs `bifrost-bootweave`
  (`springboard/src/bin/bootweave.rs`), the supervisor of `/usr/lib/bifrost/session`. Boot marks go to
  `/run/bifrost/boot-marks` (`springboard::bootsplash::mark`: `supervisor-start`, `display-handoff-start`,
  `compositor-ready`, `desktop-first-frame`, `desktop-first-ui`, `desktop-faded`, ...).
- Display: the first KMS frame redraws the loader's image (`bootsplash::paint`) and fades into the desktop over
  450 ms (`BIFROST_BOOT_FADE`). `bifrost-wm` keeps the loader's mode and fades on `shell-ready`.
- Shutdown: `ExecStopPost` parks the console on VT 12 so no text shows.
- Missing: a progress bar, timing from the previous boot, precomputed boot images and caches, on-demand services,
  pre-boot unlock, hold-until-ready, graphical failure screens, measured budgets. `docs/perf-history.jsonl` has no
  boot timings yet.

## The architecture

### Stages and owners

| Stage | Owner | Shows | Hands over |
|---|---|---|---|
| S0 firmware | UEFI (not ours) | the firmware's logo, if any | GOP framebuffer at the firmware's mode |
| S1 loader | `tools/splash-efi` | frame 0 at the native mode: background, logo, progress bar from 0 to the S1 checkpoint | the framebuffer contents and mode; the boot timing record in memory (`boot-progress` EFI variable or a config table) |
| S2 kernel and initramfs | the unified kernel image (UKI) | nothing new: deferred framebuffer-console takeover keeps frame 0; the bar holds | the inherited CRTC mode; `/run/bifrost/boot-progress` from the loader's record |
| S3 early userspace | `bifrost-bootweave` | the same frame, redrawn into the inherited KMS framebuffer, the bar continuing | the session and the display to the window server |
| S4 window server | `bifrost-wm` (`labwc` until the switch) | the same frame until desktop-ready, then one fade | the desktop |
| S5 session | the shell and the chrome clients | the desktop | |

### One frame

`bootframe` (a new `no_std` crate, `springboard/bootframe`) is the single description and renderer of the boot frame:
background colour, logo (the BifrOSt mark, scaled from the panel size), the progress bar's geometry and style, and the
failure screens' layout. The loader (GOP blit), Bootweave (KMS dumb buffer), the installer and `bifrost-wm` (first
scanout) all draw it from the same code, so the pixels are identical at every handover. Light and Dark follow the
user's appearance, stored with the boot timing record.

### The progress bar

- The bar's position is `elapsed / predicted_total`, eased, clamped below the next checkpoint, and never moves
  backwards. Checkpoints are the stage boundaries (S1 end, S2 end, S3 desktop-ready).
- `predicted_total` and the checkpoint fractions come from the previous boot's measured durations, a smoothed average
  of the last 5 boots, in `EFI/BIFROST/boot-timing` (a small versioned binary record written by Bootweave after
  `desktop-faded`). With no record (first boot, after an update) the bar uses defaults and the record is relearned.
- The bar is hidden when the whole boot is predicted to take under 1.5 s.

### Prepared, not probed

At install and on every update (`packaging/arch/recovery/system.rs`, the updater):
- A unified kernel image `EFI/BIFROST/bifrost.efi`: kernel, initramfs, command line and microcode in one signed-ready
  PE image. The loader starts it directly.
- A minimal initramfs for this machine: the modules this hardware used on the previous boot (`/proc/modules` recorded
  after desktop-ready), not a generic set; no udev coldplug walk for devices already known.
- Caches the first frame needs: the wallpaper decoded at the panel size, the Dock and menu bar icons rasterised, the
  system font's glyph atlas for the menu bar. The shell and the chrome clients load them instead of decoding at boot.

### Services on demand

Nothing starts before desktop-ready unless the first frame needs it: seatd, the window server, the shell, the chrome
clients. Every `bifrostd` service (audio, network, update, the parity of the reference's on-demand daemons) is
socket-activated and starts on its first message. systemd units that are not needed for the desktop move after
`desktop-ready` (a `bifrost-after-desktop.target`).

### Unlock is login

On encrypted installs the loader shows the unlock screen in the boot frame (the user's picture and name, a password
field), unlocks the LUKS key slot with the passphrase in EFI memory, hands the key to the initramfs through a
one-time EFI variable, and marks the session for automatic login of that user. No second login prompt, no text
console prompt.

### Reveal once

The window server keeps the boot frame on screen until **desktop-ready**: the wallpaper, the menu bar and the Dock
have each presented their first complete frame (marks from the shell and the chrome clients), with a 4 s ceiling.
Then one 450 ms fade, and the `desktop-faded` mark. No partial desktop is ever shown.

### Text-free on every path

- Kernel: `quiet loglevel=3`, deferred framebuffer-console takeover kept on, `vt.global_cursor_default=0`; initramfs
  and Bootweave log to the journal only.
- Failure screens drawn with `bootframe`: rollback in progress, "BifrOSt could not start" with Restart and Recovery,
  and after a kernel panic the next boot says the computer restarted because of a problem (pstore). Never a console.
- A held key in the loader (Shift) shows the boot options in the boot frame's style; a held key combination (Ctrl+V)
  shows the verbose boot for support.

### Symmetric shutdown

Shutdown and restart fade the desktop back to the boot frame, stop services under a 2 s budget, and leave the frame
on screen until power-off. The VT 12 parking in `springboard.service` goes away.

## Budgets (Midget, measured with `tools/boot-bench.py` on the real machine)

| Measure | Budget |
|---|---|
| Modesets after the loader | 0 |
| Frames that are not the boot frame or the desktop (text, black, partial desktop) | 0 |
| Progress bar | monotonic; no stall longer than 0.5 s while the boot is still running |
| Loader start to kernel start | under 0.3 s |
| Kernel start to Bootweave start | under 1.5 s |
| Bootweave start to desktop-ready | under 2.0 s |
| Shutdown (desktop fade to power-off) | under 2.5 s |

Firmware time (S0) is reported but not budgeted. The baseline is measured first (task 01); budgets can be tightened
from it.

## Testing

- Host: `bootframe` renders golden images (snapshot tests); the progress model is pure and tested (monotonic,
  checkpoints, defaults, smoothing); the timing record round-trips.
- Midget: `boot-bench.py installed --runs 5 --record` with per-stage times and marks; frame continuity by reading back
  the scanout framebuffer at each handover (DRM `GETFB2` and a mapped dumb buffer in Bootweave and the window server)
  and comparing it to the `bootframe` render; the journal shows no console output on tty1.

## Phases

| Phase | Outcome | Tasks |
|---|---|---|
| 1 | Baseline and continuity checks; one boot frame in every stage | 01, 02, 05 |
| 2 | The progress bar from the previous boot | 03, 06 |
| 3 | Prepared boot: UKI, minimal initramfs, caches; services on demand | 04, 07, 08 |
| 4 | Reveal once in the window server; failure screens; symmetric shutdown | 09, 11, 12 |
| 5 | Unlock is login | 10 |
