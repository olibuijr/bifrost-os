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

## One project: Bootweave

Everything that boots BifrOSt is one project, `springboard/bootweave/` (owner decision 2026-10-12, "consolidate into
one and clean up old code"):

| Part | Path | Crate or binary |
|---|---|---|
| The UEFI loader | `springboard/bootweave/efi` | `bootweave-efi` (built for `x86_64-unknown-uefi` by `tools/build-efi-boot.sh`; excluded from the host workspace) |
| The boot frame and the timing record | `springboard/bootweave/frame` | `bootweave-frame` (`no_std`, shared by every stage) |
| The early-userspace supervisor | `springboard/bootweave/supervisor.rs` | the `bootweave` binary (`/usr/bin/bifrost-bootweave`) |

The loader's artifact is `bootweave.efi` (`/usr/share/bifrost/bootweave.efi`, copied to `EFI/BIFROST/BOOTx64.efi`
and `EFI/BOOT/BOOTx64.efi`). The old splash code in `springboard/src/bootsplash.rs` (paint, fade, handoff) and the
loader's `logo.bgrx` are replaced by `bootweave-frame` (task 02) and deleted then; only the boot marks stay in the
shell. The legacy command-line files `\bifrost\cmdline.txt` and `EFI\BIFROST\cmdline.txt` are gone: installed
systems read `EFI\BIFROST\boot.cfg`, USB media `EFI\BOOT\cmdline.txt`.

**The loader is always built from source** (0.6.0 shipped a stale committed `bootweave.efi` that did not understand
the `uki=` boot line, so the firmware fell through to netboot). The committed `packaging/arch/recovery/bootweave.efi`
is a build output, not a source of truth. `tools/lab-push.sh`, `bifrost pc publish` (`tools/pc-publish.py`), the
package build (`packaging/arch/PKGBUILD`, which `package-files.sh` guards), `tools/check-iso-boot.sh` and
`tools/prepare-mkarchiso.py` all run `tools/build-efi-boot.sh release` first and fail if it fails, then
`tools/verify-efi-loader.sh` refuses any copy whose mtime is older than the newest file under
`springboard/bootweave/{efi,frame}/src`. `tools/lead-check.sh` also builds the loader for `x86_64-unknown-uefi`
(it is outside the cargo workspace) and counts its errors and warnings.

## Where BifrOSt is (2026-10-12)

- Loader: `springboard/bootweave/efi` (Rust, `no_std`, UEFI). Paints a 160 px logo through GOP, reads `EFI/BIFROST/boot.cfg`,
  picks current or rollback (`boot_policy.rs`), starts the Linux EFI stub with `initrd=... quiet splash`. It sets the
  panel's largest GOP mode: the boot's only modeset.
- Kernel: inherits the mode (simpledrm, then the GPU driver with fastboot). Initramfs hooks
  `base udev autodetect microcode modconf kms keyboard block filesystems bifrost-mounts`, busybox init.
- Early userspace: `springboard.service` (no default dependencies) runs `bifrost-bootweave`
  (`springboard/bootweave/supervisor.rs`), the supervisor of `/usr/lib/bifrost/session`. Boot marks go to
  `/run/bifrost/boot-marks` (`springboard::bootsplash::mark`: `bootweave-start`, `display-handoff-start`,
  `compositor-ready`, `desktop-first-frame`, `desktop-ready`, `desktop-first-ui`, `desktop-faded`, `shutdown-start`
  (the list is `springboard::bootsplash::CONTRACT_MARKS`); `tools/boot-bench.py` reads them).
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
| S1 loader | `springboard/bootweave/efi` | frame 0 at the native mode: background, logo, progress bar from 0 to the S1 checkpoint | the framebuffer contents and mode; the boot timing record in memory (`boot-progress` EFI variable or a config table) |
| S2 kernel and initramfs | the unified kernel image (UKI) | nothing new: deferred framebuffer-console takeover keeps frame 0; the bar holds | the inherited CRTC mode; the `BifrostBootProgress` EFI variable (efivarfs) from the loader; the supervisor republishes the bar at `/run/bifrost/bootweave/progress` |
| S3 early userspace | `bifrost-bootweave` | the same frame, redrawn into the inherited KMS framebuffer, the bar continuing | the session and the display to the window server |
| S4 window server | `bifrost-wm` (`labwc` until the switch) | the same frame until desktop-ready, then one fade | the desktop |
| S5 session | the shell and the chrome clients | the desktop | |

### One frame

`bootweave-frame` (a `no_std` crate, `springboard/bootweave/frame`) is the single description and renderer of the boot frame:
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
  after desktop-ready), reduced to an allowlist by module class; no udev coldplug walk for devices already known.
  Measured 2026-10-12 (Midget, linux-rt 7.2.9-rt3): recording every loaded module put sound, bluetooth, wifi, kvm,
  input, watchdog and sensor modules in `MODULES=`, giving an 18 MB initramfs (35 MB UKI) and 5.5 s from kernel to
  bootweave (budget 1.5 s). `boot_modules` in `system.rs` now keeps only modules whose file (`modinfo -F filename`)
  is under `drivers/{nvme,ata,scsi,block,md,gpu}`, `fs/btrfs`, `crypto`, `arch/x86/crypto` or `lib`; mkinitcpio adds
  their `depends`, `autodetect` stays. Everything else loads through udev after the root mount. The full list is
  still recorded (change detection); only the drop-in is filtered. A root on USB storage is not covered (add
  `drivers/usb/storage` there when such installs matter). The installer's own `MODULES=(btrfs virtio_gpu)` is
  already minimal and unchanged. Unverified until a Midget rebuild: image size and the stage time.

### i915 eDP probe on desktops (finding, no fix shipped)

Midget logs `[ENCODER:110:DDI A/PHY A] failed to retrieve link info, disabling eDP` and loses about 1.25 s: the
VBT declares an internal eDP panel the desktop does not have. A `video=eDP-1:d` parameter would not help. The AUX
DPCD read that times out runs in `intel_dp_init_connector` -> `intel_edp_init_connector` (drivers/gpu/drm/i915/
display/intel_dp.c) while the driver creates its encoders at probe, before any connector `force` state from `video=`
is applied (`drm_get_connector_force`, drivers/gpu/drm/drm_modes.c / drm_probe_helper.c, acts on later probing).
So it only hides a connector that has already cost the time. Not shipped. Options that remain, none vendor neutral
enough yet: a corrected VBT via `i915.vbt_firmware=` (per machine, needs the file in the initramfs), or a kernel
patch skipping eDP init when the VBT panel has no power sequencer data. Reasoned from source knowledge, not tested.

### Bench loader time

`LoaderTimeInitUSec` and `LoaderTimeExecUSec` are UTF-16 decimal strings (as written by `bootweave-efi`
`loader_var`), not binary u64. `tools/boot-bench.py` read them as u64 and reported nonsense (1407379178258.4 ms) or
nothing. It now decodes the text (`parse_loader_var`); a missing or malformed value gives NO DATA.
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
- Not text related but on every command line (installer `boot.cfg`, the loader's `DEFAULT_CMDLINE`, the ISO line):
  `threadirqs`, so the audio server can raise the sound card's IRQ thread to SCHED_FIFO
  (`docs/audio/21-lowest-latency-path.md`). The verbose transform keeps it.
- Failure screens drawn with `bootframe`: rollback in progress, "BifrOSt could not start" with Restart and Recovery,
  and after a kernel panic the next boot says the computer restarted because of a problem (pstore). Never a console.
- A held key in the loader (Shift) shows the boot options in the boot frame's style; a held key combination (Ctrl+V)
  shows the verbose boot for support.

### One mode, no console

Owner report: boot blinked and was not smooth. Midget's journal (2560x1440 DP monitor, preferred mode 59.951 Hz, also 165 Hz) showed two modesets and a console take-over:

- 4.35 s `fbcon: Taking over console` right after `Deferring console take-over`. Deferred take-over only ends when something writes to the console. The writer is not visible in the code (the cmdline is already `quiet loglevel=3`, getty@tty1 is masked, no script writes to tty1; `springboard.service` only attaches tty1 as stdin with `TTYReset`/`TTYVHangup`, which run after this point). So the cmdline now adds `fbcon=map:9`: fbcon maps to a framebuffer that does not exist and never binds to the display, whoever writes. The cost is that tty1 shows nothing on a recovery boot; use ssh or serial, or drop the token from the boot line. Unverified on hardware.
- 5.5 s Bootweave chose the fastest refresh (165 Hz) and modeset away from the firmware's 60 Hz mode. `select_mode` now picks the preferred mode (the firmware's), so `keeps_inherited` keeps it and no modeset happens. `refresh=max` in `/etc/bifrost/display.conf` restores the fastest refresh (the session script reads the same switch). Inherited and selected modes count as equal when their timings match, since `vrefresh` rounds 59.951 to 60.
- 6.9 s labwc/wlroots takes the connector's preferred mode. Bootweave now ends on that same mode, so the compositor's first commit is not a modeset. wlroots has no setting to keep the current mode; it always applies the preferred one unless an output configuration says otherwise, which is why both stages must agree on the preferred mode.

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

## Session restart after a shell stall (0.6.2 incident)

After the shell stalled (heartbeat exit 5) four times in a row, every new session ended about a second after labwc's
autostart, with no shell error in the journal, and only a reboot recovered. The chain:

1. Each shell start adds one to `/var/lib/springboard/boot_attempts` (`bare/takeover.rs`) and clears it after 60 s
   healthy. A stall exits after 20 s, so four stalls in a row left the counter at 3.
2. The next start found the counter at `MAX_FAILED_BOOTS`, wrote the reason only to `DIR/log` (not the journal) and
   exited 70 at once. The latch stays for 10 minutes (`ATTEMPTS_STALE_SECS`).
3. The session script saw the shell end, exited, and Bootweave's supervisor counted a quick failure (`after_exit`: under
   20 s). Three of those show the start-failure screen, which holds the service until Restart is chosen: that is the
   frame the lead saw and why no restart fixed it. `Session terminated, killing shell...` is `runuser` receiving
   SIGTERM when the session's cgroup was torn down.

Fixes: a stall now gives its attempt back (`takeover::forgive_stall`; a stall is not a start failure), the latch prints
to stderr as well as `DIR/log`, and the session script prints one journal line for the shell's exit status
(`shell_exit_note` in `session-policy`: 3, 5, 70 are named). Stale files under `/run/bifrost/user` were not the cause:
the script already removes `wayland-*` (which includes `wayland-0.lock`), `session.json`, `shell-ready` and `bus` before
each start.

The two noise sources in the log: `/etc/bifrost/{compositor,renderer,chrome}: No such file` came from `tr < file
2>/dev/null` (the failed redirect prints before the `2>/dev/null` applies); the script now reads them through
`session_switch`, which checks the file first. `dbus-update-activation-environment: unable to connect` was the launchers
starting before the session bus socket existed; the script now waits up to 2 s for `$runtime/bus`.
Tests: `tools/test_session_policy.py` (`session_switch`, `shell_exit_note`), `bare/takeover.rs` (`after_stall`).
Unverified on hardware.
