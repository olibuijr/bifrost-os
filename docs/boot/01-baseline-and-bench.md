# Boot 01: baseline, per-stage bench, frame continuity check

Task: `tasks/boot/01-baseline-and-bench.md`. Contract: `docs/boot-architecture.md` (Budgets, Testing).
Branch `boot/01-baseline-and-bench`, based on `main` at 5d0c9cc1.

## What is implemented

1. **Installed-boot bench on Midget** (`tools/boot-bench.py installed`). The VM and the live (installer ISO) modes are
   gone: the owner's rule is to test on Midget, and the old code paths depended on the VM and on `vm-cycle.py`.
   - Reboots through `tools/midget.py` (`cmd_reboot`, which waits for ssh and runs the update guard), then reads:
     `systemd-analyze` (firmware, loader, kernel, initrd, userspace), the loader's two Boot Loader Interface
     variables `LoaderTimeInitUSec` and `LoaderTimeExecUSec` (read from efivarfs), every mark in
     `/run/bifrost/boot-marks`, and the display-mode lines in the boot's journal.
   - Derived stages, with the formula written in the docstring:
     - `loader_ms` = ExecUSec - InitUSec (the loader's own run time, measured).
     - `kernel_to_bootweave_ms` = `bootweave-start` mark (kernel start is the clock origin, 0 ms).
     - `bootweave_to_desktop_ready_ms` = `desktop-ready` - `bootweave-start`.
     - Loader start relative to kernel start is `-loader_ms` (derived, not a separate measurement).
   - Shutdown: the previous boot's `boot-mark shutdown-start` journal line to that boot's last journal entry
     (power-off, approximated). A run's shutdown is read in the next run, so `--runs N` yields N-1 shutdown samples.
   - Medians over the runs, the contract's budget table printed with PASS, FAIL or NO DATA, and `--record` appends a
     line to `docs/perf-history.jsonl` (host `midget`, with the boot count).
   - `--no-reboot` collects only the boot Midget is in now.
   - `tools/boot-bench.py selftest` runs 11 unit tests (seconds parsing, analyze line, loader time, mark parsing,
     stage times, shutdown gap, journal timestamps, medians and budgets, flattened key names).
2. **Frame continuity checks** (`tools/boot-frames.py check`, new):
   - `modesets`: display-mode programming lines in this boot's journal (budget 0).
   - `console`: the cmdline keeps text off tty1 (`quiet`, `loglevel<=3`, `vt.global_cursor_default=0`) and no kernel
     message at crit or worse (what the console prints under `loglevel=3`) reached the journal.
   - `inherited`, `bootweave` (and any other stage given with `--stages`): the readback files from the hooks of tasks
     06 and 09 (`/run/bifrost/boot-frames/<stage>.json` and `.raw`) are compared pixel for pixel with the golden
     `boot.raw` and `boot.json` from the bootweave-frame golden test (`--golden DIR`). A stage with no readback is
     NOT MEASURED (`--strict` makes that a failure).
   - `tools/boot-frames.py selftest` runs 9 unit tests.
3. **Boot marks** (the marks the contract names, one call each):
   - `bootweave-start`: first mark in `springboard/bootweave/supervisor.rs` (replaces `supervisor-start`; the only
     consumer was the contract text, updated in `docs/boot-architecture.md`).
   - `desktop-ready`: marked next to `desktop-first-frame`. This is provisional: the contract's desktop-ready is the
     window server's three-part gate (wallpaper, menu bar, Dock), which is task 09. Until then it is the shell's
     first ready frame.
   - `shutdown-start`: `ExecStop=-/usr/bin/bifrost-bootweave mark shutdown-start` in `packaging/arch/springboard.service`,
     before the existing `ExecStopPost` console park.
   - `loader-start` and `kernel-start` are not written as marks. The loader cannot write `/run`, and kernel start is the
     origin of the clock. Both are derived as described above.
   - Every mark is also printed to the journal as `boot-mark EVENT UNIX_MS BOOTTIME_MS` (in the supervisor's `mark`),
     so the previous boot's shutdown start survives the reboot.
   - `springboard::bootsplash::CONTRACT_MARKS` lists the marks the boot code writes, and `valid_event` is the name
     check `mark` now uses. Two tests cover them.

## Map to the contract

| Contract item | Where |
|---|---|
| Budgets: modesets after the loader 0 | `boot-frames.py` `modesets`, `boot-bench.py` `modesets` |
| Budgets: no text on tty1 | `boot-frames.py` `console` |
| Budgets: loader start to kernel under 0.3 s | `boot-bench.py` `loader_ms` (measured run time; see derivation above) |
| Budgets: kernel start to Bootweave under 1.5 s | `boot-bench.py` `kernel_to_bootweave_ms` |
| Budgets: Bootweave to desktop-ready under 2.0 s | `boot-bench.py` `bootweave_to_desktop_ready_ms` |
| Budgets: shutdown under 2.5 s | `boot-bench.py` `shutdown_ms` |
| Testing: Midget bench with per-stage times and marks | `boot-bench.py installed --runs 5 --record` |
| Testing: frame continuity by readback and golden | `boot-frames.py check --golden DIR` (needs tasks 06 and 09) |
| Testing: journal shows no console output on tty1 | `boot-frames.py check` (`console` row) |

Frame progress bar, the timing record, caches and the window server's gate are other tasks; nothing here changes them.

## Not done here (todo_left)

- **No Midget run.** This worktree has no Midget access, so the baseline numbers are not measured and
  `docs/perf-history.jsonl` has no boot line yet. The lead records them (see the commands below), then writes the
  measured medians into the contract's Budgets section.
- The readback hook that writes `/run/bifrost/boot-frames/<stage>.{json,raw}` is task 06 (Bootweave) and task 09
  (window server). `boot-frames.py` reports NOT MEASURED until then.
- The golden `boot.raw` and `boot.json` come from the bootweave-frame golden test (task 02); its `render` is still
  `todo!()` in this branch.
- `desktop-ready` is provisional (see above; task 09).
- The loader's `loader-start` is derived. If the lead wants a measured value, the loader would have to publish its
  start TSC in the boot-timing record (task 03).
- The bench's `--runs` default is 5, as the task's Done-when asks for five boots.
- `springboard/src/bootsplash.rs` (`mark` only): the only change is the name check and the list. Nothing else.

## Review checks (by reading; nothing was compiled or run)

Per the owner's rule for wave agents, no compiler, cargo or Python test run was made in this worktree. The checks are:

- The unit tests in `tools/boot-bench.py` (11) and `tools/boot-frames.py` (9) and the new `bootsplash` tests (2) were
  worked out by hand against the code. The lead runs `python3 tools/boot-bench.py selftest` and
  `python3 tools/boot-frames.py selftest` after the merge.
- Signatures used from `springboard/bootweave/supervisor.rs` (`springboard::bootsplash::boottime_ms() -> f64`,
  `springboard::bootsplash::mark`) and `tools/midget.py` (`ssh`, `state`, `cmd_reboot`, `SSH`) were read from source.
- `python3 tools/check-names.py`: exit 0, no hits.
- Fixed in this round: the loader's `LoaderTimeInitUSec` and `LoaderTimeExecUSec` are read with `od -An -tu8 -j4 -N8`
  (efivarfs values are a little-endian u64 after 4 attribute bytes; the earlier `tail | tr` read gave no loader time).
  A shutdown sample is attached only when the previous run in the same bench succeeded.
- `springboard/Cargo.lock` is reverted: no dependency was added, and cargo updates the lock for the new workspace
  member on the lead's build.

## Midget verification (for the lead; not run by the agent)

Prerequisites: the build under test is published (`bifrost pc publish`) and Midget has it
(`bifrost pc midget ota` until `phase=accepted`, then `python3 tools/midget.py recovery`), and Midget is in the
installed system (`python3 tools/midget.py state` prints `installed`).

```sh
# five timed reboots into the installed system, medians recorded in docs/perf-history.jsonl
python3 tools/boot-bench.py installed --runs 5 --label "boot-01 baseline" --record

# the last boot only (no reboot): per-stage times and the budget table
python3 tools/boot-bench.py installed --no-reboot

# frame continuity on the last boot: modesets, console text, and any readback stages
python3 tools/boot-frames.py check --golden /path/to/golden-dir
python3 tools/boot-frames.py check --stages inherited,bootweave,desktop-first --golden /path/to/golden-dir --strict

# screen at the desktop, for the record
python3 tools/midget.py screen out/boot-01-desktop.png
```

Expected today: `boot-bench` reports every stage that has marks; `bootweave_to_desktop_ready_ms` is the shell's first
frame; `shutdown_ms` appears from the second run onward (the `shutdown-start` mark needs this branch's
`springboard.service` on Midget, so the first run after the update has no shutdown sample). `boot-frames` shows
`NOT MEASURED` for the readback stages and `FAIL` only for `modesets` or `console` if the boot is not text-free.
