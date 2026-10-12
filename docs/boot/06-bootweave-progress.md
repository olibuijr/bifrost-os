# Bootweave continues the bar and writes the timing record (boot task 06)

Contract: `docs/boot-architecture.md` ("The progress bar", "Stages and owners", "Testing"). Owner file:
`springboard/bootweave/supervisor.rs` (the `bootweave` binary, `/usr/bin/bifrost-bootweave`). Depends on
`bootweave-frame` (task 02 renderer, task 03 progress model and record codec).

## What is implemented

1. **The bar in the inherited framebuffer.** At supervisor start a bar thread (`bar_loop`) opens `/dev/fb0`
   (the inherited framebuffer, through its sysfs description `/sys/class/graphics/fb0`: `virtual_size`,
   `bits_per_pixel`, `stride`). It seeds a `bootweave_frame::Progress` from the loader's record (`seed`: the bar
   stands at the loader checkpoint, the loader and kernel stages are done, so it continues instead of jumping).
   Every 33 ms it renders the bar with `render_bar` (BGRX, the framebuffer's order) and writes only the bar rectangle
   rows (`bar_rect`) to the device. No DRM master, no modeset, no `unsafe`: the window server keeps the display.
   The bar stops when the window server's socket exists (`wayland-0`, the handover), when `shell-ready` appears, or
   after a 60 s ceiling.
2. **Hand-off input.** `/run/bifrost/boot-progress` is the loader's record (`BootTiming::decode`, `BBT1`). The
   `loader_ms` field is this boot's measured loader time; the other fields and the appearance are the record the
   loader used. `pick_timing` chooses hand-off, then the ESP record, then `BootTiming::DEFAULT`.
3. **Timing record after `desktop-faded`.** `record_when_faded` polls `/run/bifrost/boot-marks` (up to 120 s).
   When `desktop-faded` appears it measures this boot (`measure`): kernel = `supervisor-start` (kernel start is
   time zero), userspace = `desktop-ready`, or `desktop-faded` when that mark is absent (no code emits
   `desktop-ready` yet). The record is `next_record(previous, measured, appearance)`: `BootTiming::add_sample` folds
   the boot in (running mean over at most 5 boots). It is written atomically (temporary file, `sync_all`, rename) to
   `/boot/EFI/BIFROST/boot-timing`. A missing loader time keeps the previous loader average.
4. **Module list for task 04.** The same thread writes `/boot/EFI/BIFROST/boot-modules` from `/proc/modules`
   (`module_list`: names sorted and unique, one per line), only when it differs from the file.
5. **Readback hook (task 01).** If `/run/bifrost/bootweave/readback-request` exists when the bar stops, the whole
   framebuffer is copied to `/run/bifrost/bootweave/readback.bgrx` and its geometry (`W H STRIDE_PX bgrx`) to
   `readback.txt`. The continuity check compares this with the `bootframe` render of the same frame.
6. The marks and the existing `session-*` / `compositor-ready` / `desktop-first-frame` behaviour are unchanged.

## Changes outside the owned file

- `springboard/Cargo.toml`: `bootweave-frame = { path = "bootweave/frame" }` in `[dependencies]` (a workspace path
  dependency, no registry crate). The supervisor needs the frame crate; springboard did not depend on it before.

## Pure logic and tests (`#[cfg(test)] mod tests` in supervisor.rs)

| Test | What it pins |
|---|---|
| `marks_keep_the_first_of_each_event_and_skip_junk` | first mark wins; junk, negative lines skipped |
| `measure_splits_kernel_from_userspace_at_bootweave_start` | 412.3 and 2600.0 give kernel 412, userspace 2188 |
| `measure_falls_back_to_faded_and_rejects_incomplete_marks` | faded fallback (3000.0 - 412.3 = 2588); missing or reversed marks give `None` |
| `the_next_record_folds_this_boot_in_and_takes_the_chosen_appearance` | first sample: samples 1; 5-boot window; loader kept when not measured |
| `the_timing_for_this_boot_prefers_the_loader_hand_off_then_the_record_then_the_defaults` | precedence; defaults total 3800 ms |
| `the_module_list_is_sorted_unique_and_one_per_line` | `/proc/modules` parsing and file text |
| `the_elapsed_clock_adds_the_loader_time_and_never_goes_negative` | 300 + 1234.6 = 1535 |
| `the_framebuffer_size_and_the_readback_header_are_exact` | `1920,1080` only; header text |

One test (`the_next_record_folds_this_boot_in_and_takes_the_chosen_appearance`) calls `BootTiming::add_sample`, which
is `todo!("boot-03")` in this worktree. It is expected to pass once boot-03's body is merged, and it assumes the
running mean returns the sample itself for the first boot. Not run here (the task forbids `cargo test`).
`seed` (not tested) calls `Progress::checkpoint`, also a boot-03 stub, so the supervisor panics at boot until it lands.

## Build check

Not run by this agent: the workspace's `no-agent-builds` hook blocks `cargo check` for task agents (the lead runs
all builds after the waves merge). The change was reviewed by reading it: every import is a `pub` item of
`bootweave-frame` (`render::{bar_rect, render_bar, PixelOrder}`, `{Appearance, BootTiming, Progress, Stage}`) or of
springboard (`bootsplash::{MARKS, boottime_ms}`); the only I/O is `std::fs` and writes to `/dev/fb0`, with no `unsafe`.
The lead's check: `CARGO_TARGET_DIR=<private dir> cargo check --offline -j 2 -p springboard --tests` from
`springboard/`, then `cargo test -p springboard supervisor` once the boot-02/03 bodies are merged.
`python3 tools/check-names.py`: clean.

## Left to do (todo_left)

- `BootTiming::add_sample`, `encode`, `decode`, `Progress::at`, `Progress::checkpoint`, `render_bar` are boot-02/03
  bodies. Until they land, a run of the supervisor panics at the first `seed` call (the bar thread).
- The loader must write `/run/bifrost/boot-progress` (or the initramfs must copy the loader's record there from its
  hand-off), with `loader_ms` set to the measured loader time. Owner: task 03 and the initramfs hook.
- `desktop-ready` is not emitted by any component yet (task 09 reveal-once). Until it is, userspace uses
  `desktop-faded`.
- `/boot` must be mounted read-write when the record is written (the installed fstab does this; check on Midget).
- The continuity hook's request file is created by the baseline tool (task 01); nothing creates it here.
- Frame continuity between the loader and this bar is checked on Midget (below), not on the host.

## Midget verification (for the lead)

Run on Midget after `bifrost pc publish` and `bifrost pc midget ota` (`phase=accepted`). Lead's commands:

```sh
# 1. the bar is on the inherited framebuffer and advances (boot-marks has the stage marks)
ssh midget 'cat /sys/class/graphics/fb0/virtual_size /sys/class/graphics/fb0/stride /sys/class/graphics/fb0/bits_per_pixel'
ssh midget 'journalctl -b -u springboard.service --no-pager | grep -i "bifrost-bootweave" | head'   # no "bar not drawn"
# 2. continuity: request a readback, reboot, collect and compare against the bootframe render
# /run is tmpfs: the request must be created after boot, before the handover finishes, by the baseline tool (task 01).
# The supervisor writes /run/bifrost/bootweave/readback.{bgrx,txt} when the bar stops and the request exists.
python3 tools/midget.py recovery                          # expect phase=accepted
# 3. the timing record changes after each boot
ssh midget 'od -A d -t x1 /boot/EFI/BIFROST/boot-timing'  # note the bytes, reboot, compare: samples and values move
ssh midget 'cat /boot/EFI/BIFROST/boot-modules | head'    # the modules of this boot
ssh midget 'cat /run/bifrost/boot-marks'                  # supervisor-start, desktop-faded present
```

Expected on Midget: the bar in the same place across the loader and Bootweave handover (no jump); the
`boot-timing` file is 24 bytes with magic `BBT1`, `samples` at most 5, and changes after each boot.
