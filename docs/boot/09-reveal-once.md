# 09 Reveal once: the window server holds the boot frame until desktop-ready

Contract: `docs/boot-architecture.md`, "Reveal once" (the boot frame stays on screen until the wallpaper, the menu bar
and the Dock have each presented a complete frame, with a 4 s ceiling; then one 450 ms fade and `desktop-faded`).
Task: `tasks/boot/09-reveal-once.md`. Branch `boot/09-reveal-once`.

## What is implemented

- `springboard/src/bootreveal.rs` (new, pure, no I/O, shared by path with the window server):
  - `DESKTOP_PARTS`: the marks that gate the reveal, `wallpaper-first-frame`, `menu-bar-first-frame`, `dock-first-frame`.
  - `chrome_part(id)`: the menu bar (chrome id 0) and the Dock (chrome id 2) map to their marks; other chrome writes none.
  - `Parts`: which parts have been seen, read from marks text (`"<ms> <event>"` lines; exact event names only).
  - `Reveal`: the gate. `poll(now, marks)` re-reads the marks file at most every 100 ms, records `ready_at` once when
    all three parts are seen or 4000 ms have passed since the gate started, and stops reading after that.
- `springboard/wm/src/boot.rs`: includes `bootreveal.rs` by path, re-exports `Reveal`, adds `read_marks_file()`. The
  fade (`splash_alpha`, 450 ms smoothstep) is unchanged and its tests now cover the hold-then-fade order.
- `springboard/wm/src/drm.rs` (window server frame loop): `Boot` owns a `Reveal` and a start `Instant`. `alpha()`
  polls the gate each frame, returns 1.0 (boot frame opaque over the desktop) while held, marks `desktop-ready` at the
  first ready poll, then runs the single fade and marks `desktop-faded` as before. The old `shell-ready` file check is gone.
- `springboard/src/bare/kms.rs` (labwc path, `BIFROST_BOOT_FADE`): `boot_fade` carries a `Reveal`. While held, `present`
  writes the boot frame with the bar at its hand-off fill (`Handoff::hold`, new in `bootsplash.rs`) and does not start
  the hand-off. `desktop-first-ui` is marked when the hand-off starts, after the gate opens.
- `springboard/src/bare/wayland.rs` (the shell's ready marks, minimal):
  - `mark_first_frame` (first desktop surface commit) writes `wallpaper-first-frame`, and still writes `shell-ready`.
  - `mark_chrome_frame(id)` (first commit of a chrome surface, in both the GPU and the shm path) writes the menu bar's
    or the Dock's mark. Each part is written at most once per process (a bit per chrome id).
  - Correction to the earlier draft: the menu bar and the Dock are separate layer surfaces, not part of the desktop
    surface, so they are marked on their own commits, not on the desktop's first commit.

## Mapping to the contract

| Contract line | Where |
|---|---|
| Keep the boot frame until wallpaper, menu bar, Dock each present | `Reveal::poll`, `DESKTOP_PARTS`, `chrome_part`, the marks in `wayland.rs` |
| 4 s ceiling | `READY_CEILING_MS` (from the gate start, not from boot) |
| One 450 ms fade, then `desktop-faded` | `splash_alpha` in `wm/src/boot.rs`, `drm.rs` `alpha()`; `kms.rs` `present` through `Handoff::compose` |
| No partial desktop shown | the gate holds the frame until all three parts are marked |
| Same rule on the labwc path | `kms.rs` `boot_fade` with `Reveal` |

## Tests (hand-worked, not run: the lead builds)

- `bootreveal.rs`: `one_part_is_not_enough`, `all_three_parts_complete_the_desktop`,
  `unrelated_marks_and_malformed_lines_are_ignored`, `parts_accumulate_across_reads`,
  `the_gate_holds_until_every_part_is_marked`, `ready_at_is_recorded_once_and_sticks`,
  `the_ceiling_releases_the_frame_after_4_s`, `the_ceiling_counts_from_the_gate_start_not_from_boot`,
  `the_marks_file_is_read_at_most_every_100_ms`, `only_the_menu_bar_and_the_dock_are_chrome_parts`, `no_reads_after_ready`.
- `wm/src/boot.rs`: `the_loader_frame_holds_until_the_shell_is_ready`,
  `it_fades_with_the_loaders_easing_and_then_goes_away`, `the_frame_is_held_until_the_last_desktop_part_then_fades_once`,
  `a_stalled_desktop_is_revealed_by_the_ceiling`.
- Not added: a KMS unit test (the held branch needs a `Display`), so the labwc path is verified on Midget only.

## What is left

- The held frame is still drawn from the shell's `bootsplash.rs`. When task 02 (`bootweave-frame`) lands, the window
  server and the KMS path should draw the held frame from `bootweave_frame` and drop `bootsplash.rs`'s paint code.
  Pixel layout is the same today, so this is a swap, not a behaviour change.
- The marks are coarse: the menu bar and the Dock are marked on their first commit, not on the first frame that shows
  the complete bar contents. If Midget shows a flash of an empty bar or Dock, add a mark from the chrome client's
  first real draw.
- `springboard/Cargo.lock` is not changed here (no dependency added). The lock file is not touched by this branch.
- `wm` has no build-time dependency on `bootweave-frame`; it includes `bootsplash.rs` and `bootreveal.rs` by path.

## Midget verification steps (for the lead, after the merge build)

1. Install the built shell and window server on Midget as usual (`bifrost pc midget ota`, then reboot). The reveal only
   runs where `BIFROST_BOOT_FADE` is set in the environment of the compositor (the code checks it in `wm/src/drm.rs` and
   `bare/kms.rs`); confirm how the session passes it before the boots, since this branch does not change that wiring.
2. Boot five times. After each boot run `ssh midget 'cat /run/bifrost/boot-marks'`.
3. Expect, in this order and each once per boot: `supervisor-start`, ..., `wallpaper-first-frame`,
   `menu-bar-first-frame`, `dock-first-frame`, `desktop-ready` (no earlier than the last part), `desktop-first-ui`,
   `desktop-faded`. `desktop-faded` minus `desktop-ready` is about 450 ms (within one progress tick).
4. Expect `desktop-ready` to come before `desktop-faded` by the fade only; a `desktop-ready` at about 4000 ms after the
   gate start means a part was missing and the ceiling released the frame. Record that boot as a failure.
5. Frame check: `tools/boot-frames.py` (named by the task's Done when; not in the tree yet, task 01 owns it) must see no
   partial-desktop frame over the five boots.
6. Take a Midget screenshot right after `desktop-faded` (`bifrost pc midget screen shot.png`) and compare it with the
   desktop; take one screenshot during the hold from the loader's own frame to confirm the bar is at its hand-off fill.
