# Boot 05: kernel and early userspace never show text

Task: `tasks/boot/05-text-free-kernel.md` (git-ignored). Contract: `docs/boot-architecture.md`, "Text-free on every path".

## What is implemented

1. **One text-free command line, four writers.** The same flags are set in every place that writes a command line:
   - `springboard/src/installer/plan.rs` (`cmdline_with`): the installed `EFI/BIFROST/boot.cfg` and the USB
     `EFI/BOOT/cmdline.txt`. Adds `udev.log_level=3` (the system udev, not only the initramfs one) and
     `systemd.log_target=journal` (PID 1 log lines go to the journal, not the console).
   - `tools/prepare-mkarchiso.py` (`_make_bootmode_uefi.bifrost`): the ISO's `EFI/BOOT/cmdline.txt` and
     `EFI/BIFROST/boot.cfg`. It got the same two tokens in this task. Debug ISOs still override the line with
     `bifrost-cmdline` (journal diagnostics and verbose status on purpose), which is left as is.
   - `springboard/bootweave/efi/src/cmdline.rs` (`DEFAULT_CMDLINE`): the loader's fallback when neither file is
     readable. Same flags. `splash` is removed.
   - Kept from before: `quiet loglevel=3 systemd.show_status=false rd.systemd.show_status=false rd.udev.log_level=3
     vt.global_cursor_default=0 systemd.tty.rows.console=`, and the hardware flags.
2. **Ctrl+V for one verbose boot.** `springboard/bootweave/efi/src/main.rs` (`verbose_requested`) drains at most 32
   queued keystrokes from `SimpleTextInputEx` without waiting. Ctrl+V (control character 0x16, or `v`/`V` with a
   Control shift bit) sets verbose for that boot only. `cmdline::verbose` then removes the quiet tokens and adds
   `loglevel=7 systemd.show_status=true rd.systemd.show_status=true systemd.log_level=info udev.log_level=info`.
   Nothing is stored, so the next boot is quiet again. The rollback line gets the same transform when it is used.
3. **systemd status and logs off the console.** `packaging/arch/systemd/50-bifrost-console.conf` installs as
   `/usr/lib/systemd/system.conf.d/50-bifrost-console.conf` (`ShowStatus=no`, `LogTarget=journal`); it is wired in
   `packaging/arch/package-files.sh` beside the existing `getty@tty1` mask. The kernel command line overrides it,
   which is what makes the verbose boot visible.
4. **Tests.** `cmdline.rs` has 7 tests (default flags, verbose transform worked by hand, idempotence, token matching,
   whitespace). `plan.rs` `config_files` now asserts every baseline flag and that `splash`/`plymouth` are absent.

## Mapping to the contract

| Contract line | Where |
|---|---|
| Kernel: `quiet loglevel=3`, `vt.global_cursor_default=0`, deferred takeover kept on | cmdline (points 1 and 2); fbcon takeover is the kernel default, see "Not done" |
| initramfs and Bootweave log to the journal only | `rd.udev.log_level=3`, `rd.systemd.show_status=false`; Bootweave's own logging is task 06 |
| Failure screens never show a console | not this task (task 11) |
| Ctrl+V holds verbose for that boot only | point 2 |
| systemd and udev status off on the console | point 3 |

## Not done here (todo_left)

- **fbcon deferred takeover is not verified in the repo.** No kernel config fragment exists in this tree, and nothing
  passes `fbcon=nodefer`. The default is expected on, so the lead must confirm it on Midget (below). Do not add
  `nodefer`.
- **The loader's key check depends on firmware.** Only keystrokes already in the firmware queue are seen; a key
  released before the loader starts is not. Whether Ctrl+V arrives as 0x16 or `v` plus a Control bit varies by
  firmware. Midget will show which.
- **Nothing in this task removes the `ExecStopPost` VT 12 parking** in `springboard.service`; that is task 12.
- **The loader's module header still says "boot splash"** and its logo code is replaced by task 02.
- **The bench and frame scripts** (`tools/boot-bench.py`, `boot-frames.py`) are not written by this task.

## Midget verification (lead)

Run after the update is installed and the machine has booted at least once with the new command line.

1. Command line is the new one on the installed system:
   `ssh midget 'cat /proc/cmdline'`
   Expect `loglevel=3`, `quiet`, `systemd.show_status=false`, `udev.log_level=3`, `systemd.log_target=journal`,
   `vt.global_cursor_default=0` and no `splash`.
2. Systemd manager drop-in is installed and active:
   `ssh midget 'ls /usr/lib/systemd/system.conf.d/50-bifrost-console.conf && systemd-analyze dump | grep -i "ShowStatus\|LogTarget" | head'`
3. Deferred framebuffer takeover is on in the running kernel (needs `CONFIG_IKCONFIG_PROC`; otherwise look in
   `/boot` for the config file):
   `ssh midget 'zcat /proc/config.gz | grep -E "FRAMEBUFFER_CONSOLE_DEFERRED_TAKEOVER|^CONFIG_FRAMEBUFFER_CONSOLE="'`
   Expect the deferred option `=y`. Do not change the kernel config in this task.
4. Five clean boots with no console output on tty1 (the task's done-when). For each boot:
   `python3 tools/midget.py recovery` (must say `phase=accepted`), then `bifrost pc midget screen /tmp/boot-N.png`
   and inspect the picture for text.
5. Failed-unit boot: make one unit fail on purpose (for example `ssh midget 'systemctl mask --now bifrost-privd.service'`
   with a failing `ExecStart`, or a test unit that exits 1), reboot, and repeat step 4. Restore the unit afterwards.
   The journal must still hold the failure: `ssh midget 'journalctl -b -p err --no-pager | head'`.
6. Ctrl+V boot: hold Ctrl+V at power-on, watch the console for kernel and systemd lines, then boot normally and confirm
   step 1 shows the quiet line again.
7. Host tests for the new pure logic (not run by this agent):
   `rustc --edition 2021 --test springboard/bootweave/efi/src/cmdline.rs -o /tmp/cmdline-tests && /tmp/cmdline-tests`
   and `cargo test --offline -p springboard installer::plan` from `springboard/`.

## Build checks

- `CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents-boot-05 cargo check --offline -j 2 -p springboard --tests` (from `springboard/`)
- `CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents-boot-05 cargo check --offline -j 2 --manifest-path bootweave/efi/Cargo.toml --target x86_64-unknown-uefi` (from `springboard/`)
- `python3 tools/check-names.py` (owner name rule)
