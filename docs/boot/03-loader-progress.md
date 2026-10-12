# Boot task 03: loader progress, timing record, options

Task file: `tasks/boot/03-loader-progress.md` (git-ignored). Contract: `docs/boot-architecture.md`, "Stages", "One frame",
"The progress bar", "Budgets". Branch `boot/03-loader-progress`, built on `main` at 5d0c9cc1 (the bootweave skeleton).

## What is implemented

### `springboard/bootweave/frame` (`bootweave-frame`, owner of `progress.rs` and `timing.rs`)

- `timing.rs`: the `EFI/BIFROST/boot-timing` record (`BBT1`, 24 bytes, little endian, FNV-1a 32 checksum over bytes
  0..20, reserved bytes must be zero, every stage 1..=600 000 ms, samples 0..=5, appearance 0 Dark or 1 Light).
  - `add_sample`: `(old * n + sample) / (n + 1)` with `n = min(samples, 4)`. The first measured boot replaces the
    defaults; from the fifth boot the weight of a new boot is 1/5. The sample count stops at 5.
  - `stage_end_ms(stage)`: the predicted end of each stage since loader start.
  - `Handoff`: the 32-byte value of the `BifrostBootProgress` variable: the 24-byte record, then `loader_ms` (u32 LE at
    24) and `shown_permille` (u32 LE at 28, at most 1000).
  - Tests: FNV-1a vectors (`""` and `"a"`), the default record's exact bytes, round trip, rejection of a bad magic,
    length, checksum, samples, appearance, zero or oversized stages, the first sample, averaging of the second sample,
    the 5-boot settle (`(100*4+200)/5 = 120`, then `(120*4+200)/5 = 136`), stage ends, and the hand-off round trip.
- `progress.rs`: the progress model.
  - Each stage owns a segment of the bar: `Loader` 0 to `loader/total`, `Kernel` to `(loader+kernel)/total`,
    `Userspace` to 0.98 (the last 2 % is the done mark; 1.0 only after `stage_done(Userspace)`).
  - Inside the running stage the position follows the stage's predicted duration: 90 % of the segment at the predicted
    end. Past that it creeps, `0.9 + 0.1 * (x - 1) / x` of the segment for `x` durations elapsed, so a slow stage keeps
    moving and never reaches its own end before `stage_done`.
  - `stage_done(s)` snaps the bar up to `checkpoint(s)`. `shown` never moves back.
  - `visible()` is false for predicted totals under 1.5 s.
  - Tests: checkpoints, the loader segment (`0.45 * 3/38` at 150 ms, `0.9 * 3/38` at 300 ms), monotonicity, overrun
    creep below the checkpoint (`3000 ms` and `9000 ms`), snapping and the kernel segment (exact formula at 2500 ms),
    the visibility threshold (1499 and 1500 ms), completion, and the Userspace hold (exact value at 3800 ms, bounded at
    very large times).

### `springboard/bootweave/efi` (`bootweave-efi`, owner of the loader)

- `Cargo.toml`: `bootweave-frame = { path = "../frame" }` (no new registry crates; `Cargo.lock` gains one path entry).
- `src/main.rs`:
  - Reads `EFI\BIFROST\boot-timing` (up to 64 bytes). A missing or invalid record falls back to `BootTiming::DEFAULT`,
    and the frame takes the record's appearance.
  - Frame 0 is drawn at the panel's native GOP mode (the one modeset, as before) with `Frame { screen: Boot, progress }`.
    The whole frame is rendered by `bootweave_frame::render` into a `u32` copy (`PixelOrder::Bgrx`) and blitted with
    `BufferToVideo`. The bar is hidden when the predicted total is under 1.5 s.
  - Before the kernel stub starts, the loader marks the loader stage done, redraws the bar (only the bar rectangle,
    through `render_bar` and `bar_rect`), drops the GOP protocol, and sets the `BifrostBootProgress` variable (vendor
    GUID `d1f0b07c-5b2e-4a8d-9c3f-6e4a2b7d9f10`, attributes BOOTSERVICE and RUNTIME access, 32 bytes as above).
  - Shift options: at loader start the keys already buffered are drained with `SimpleTextInputEx`. If one of them
    carries a Shift bit (`RIGHT_SHIFT` 0x1, `LEFT_SHIFT` 0x2 of `KeyShiftState`), the options screen opens:
    Current, then Previous (`rollback.txt`) and Recovery (`recovery.txt`) when those files hold a line. Up and Down move
    the selection, Enter boots it. The drain never waits, so the loader's time is not spent on the check.
  - The loader's `paint()` and `logo.bgrx` are gone (`git rm`); the mark is drawn by `bootweave-frame` (task 02).
  - Boot policy (pending marker, one attempt, rollback) is unchanged.
- The UEFI check passes: `cargo check --offline --manifest-path bootweave/efi/Cargo.toml --target x86_64-unknown-uefi`.

## How it maps to the contract

| Contract | Here |
|---|---|
| S1 loader draws frame 0 at the native mode with the bar from 0 | `Session::first_frame`, `Display::open` (native mode, unchanged) |
| Progress from the previous boot's durations, checkpoints at the stage ends | `Progress::new(BootTiming)` with `BootTiming` read from `EFI/BIFROST/boot-timing`, defaults without a record |
| The bar is hidden under 1.5 s predicted | `Progress::visible` |
| The bar never moves backwards; clamped below the checkpoint until done | `Progress::at` (monotonic, segment-bounded, snaps on `stage_done`) |
| Hand the record and the loader's state to the kernel side | `BifrostBootProgress` variable (`Handoff`) |
| Prepared-not-probed, text-free | Nothing printed; the loader does no probing beyond the existing file reads |
| Shift opens the boot options in the frame's style | `shift_at_start`, `choose_option`, `Screen::Options` |

## Contract changes (for the lead)

1. The hand-off variable is named `BifrostBootProgress` in the task file; `docs/boot-architecture.md` says
   "`boot-progress` EFI variable or a config table". Proposed: the variable `BifrostBootProgress` with the vendor GUID
   `d1f0b07c-5b2e-4a8d-9c3f-6e4a2b7d9f10` and the 32-byte layout above. Task 06 reads it with the same GUID.
2. The `Userspace` segment ends at 0.98 until `stage_done(Userspace)`, and the creep below the checkpoint replaces the
   "clamped below the next checkpoint" wording of "The progress bar" (the rule is now: never reach the running stage's
   checkpoint before `stage_done`).
3. A new optional file `EFI\BIFROST\recovery.txt` (a command line, like `boot.cfg`) backs the Recovery option. The
   installer has to write it, or the option is absent. Not yet written by anything.
4. The loader no longer contains the logo; `logo.bgrx` is deleted here. Task 02 draws the mark in `bootweave-frame`.

## Todo left

- Animation during the loader itself. The loader does not wait for the bar (its budget is loader start to kernel start
  under 0.3 s), so the bar is drawn at the moments the loader is in control and then holds through S2 at the handed-off
  position. A periodic GOP timer event could animate the bar during the kernel stub's read, but the stub blocks inside
  `start_image`, so that would need Bootweave or a separate thread. The contract's "the bar holds in S2" covers the
  gap; the continuous motion starts in Bootweave (task 06).
- Shift detection depends on the firmware reporting the shift bit with a key event. A Shift press alone may not produce
  an event on every keyboard or firmware; on Midget, check this first (see steps).
- Ctrl+V verbose boot is not implemented (no owner in task 03; the contract puts it in the loader's key handling).
- The record's appearance is only read. Its writer (and the user's Light or Dark choice) is task 06.
- No host test runs the loader (it is `no_std` for the UEFI target, excluded from the host workspace). Its logic lives
  in `bootweave-frame`, which has the tests.
- `cargo test` was not run in this worktree, per the task rules; run the tests with the lead's build.

## Verification

Host (done in this worktree, private target dir):

```
cd springboard
CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents-boot-03 cargo check --offline -j 2 -p bootweave-frame --tests
CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents-boot-03 cargo check --offline -j 2 --manifest-path bootweave/efi/Cargo.toml --target x86_64-unknown-uefi
```

Both pass. The name guard was not run in full: `tools/check-names.py` walks the whole springboard tree (including
the assets symlink) and took more than two minutes here. A targeted scan of the changed files found no hits:
`grep -inP "(?<![A-Za-z0-9])(apple|mac ?os|finder|safari|spotlight|appkit|uikit|ipad|iphone|ipados|xcode|siri|icloud|cupertino)" springboard/bootweave/frame/src/progress.rs springboard/bootweave/frame/src/timing.rs springboard/bootweave/efi/src/main.rs springboard/bootweave/efi/Cargo.toml`
returns nothing. The lead should run `python3 tools/check-names.py` once after the merge.

Midget (lead, after `tools/build-efi-boot.sh` and install):

1. Default record, so the loader uses 0.3 / 1.5 / 2.0 s (the defaults) with the bar visible. Write it as the ESP file
   `EFI/BIFROST/boot-timing` on the ESP (copy the printed file there; the ESP mount point on Midget is whatever `bifrost pc midget state` reports; the file is
   the 24 bytes below):

   ```
   printf '\x42\x42\x54\x31\x2c\x01\x00\x00\xdc\x05\x00\x00\xd0\x07\x00\x00\x00\x00\x00\x00\xb3\x2d\xea\x71' > /tmp/boot-timing
   ```

   The bytes are `BBT1`, loader 300, kernel 1500, userspace 2000, samples 0, Dark, reserved, checksum `0x71ea2db3`.
2. Reboot the machine (`bifrost pc midget update --restart` or `ssh midget reboot`) and take a screen capture on the
   first frame: `bifrost pc midget screen <scratchpad>/boot03-frame0.png`. The bar must be at
   the left edge of its track, at about 0 %, and the logo centred at the native mode.
3. Read the hand-off after the kernel is up:
   `ssh midget 'sudo dd if=/sys/firmware/efi/efivars/BifrostBootProgress-d1f0b07c-5b2e-4a8d-9c3f-6e4a2b7d9f10 bs=1 skip=4 2>/dev/null | xxd'`.
   Expected: 32 bytes, the first four `42 42 54 31`, loader_ms at 24 about 1 to 20 ms, shown_permille at 28 is 78
   (the loader checkpoint, 300 / 3800 = 7.9 %, truncated). A corrupt record is a failed check.
4. Shift: power on, hold Shift and press a key during the firmware screens (before the loader), then look for the options
   screen (Current, Previous if `rollback.txt` exists). Choose with Down and Enter. Without a buffered key the screen
   does not open (see todo_left). Record the result in the report.
5. Missing or corrupt record: delete `EFI/BIFROST/boot-timing` on the ESP, boot, and confirm the frame still shows with the
   bar (defaults). Then the record is relearned by task 06.
6. `python3 tools/midget.py recovery` must still say `phase=accepted` after the update.
