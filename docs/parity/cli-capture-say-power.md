# Parity: screencapture, say, caffeinate, pmset (command-line tools)

Slug: `cli-capture-say-power`. Code: `springboard/cmd` (package `bifrost-cmd`), one binary per tool in `cmd/src/bin/`,
logic in `cmd/src/<tool>.rs` with unit tests. Status: code written and committed, not built and not checked on Midget.
Nothing here is `parity` yet; the rows move to `partial` at most until the Midget checks below pass.

## Evidence used

- Man pages, from the shipped macOS 27 extract (clean room, names and flags only):
  - `~/Projects/macos-ref/extract/ref27/usr/share/man/man1/screencapture.1`
  - `~/Projects/macos-ref/extract/ref27/usr/share/man/man8/caffeinate.8`
  - `~/Projects/macos-ref/extract/ref27/usr/share/man/man1/pmset.1`
- `say.1` is listed in `ref27-index.tsv` but was not extracted, so the `say` option set is what the desktop can honour,
  not a full read of the page. Re-check once `say.1` lands under `ref27/`.
- The binaries (`usr/sbin/screencapture`, `usr/bin/say`, `usr/bin/pmset`, `usr/bin/caffeinate`) are not extracted, so
  no REA evidence ids were taken for this slice.
- BifrOSt code read: `springboard/src/bare/control.rs` (the `shot` request returns a raw `RIMG` frame at full
  resolution, `shot 1`), `src/shell/prefs.rs` and `src/bin/shell.rs` (settings file `settings.ini` in the data
  directory), `src/bare/sleep.rs` (`LidInhibitor` takes a logind block through `systemd-inhibit`),
  `src/shell/options.rs` (`system_sleep_minutes`), `src/pngenc.rs` (the PNG writer).

## screencapture

- API: the macOS screen-capture command line (`screencapture [options] [file]`).
- BifrOSt path: `cmd/src/screencapture.rs`, `cmd/src/bin/screencapture.rs`. Uses the control socket `shot 1` for the
  whole panel, the shell's PNG writer (`springboard::pngenc`), and `wl-copy --type image/png` for `-c`.
- Done: full screen to a PNG file; `-R x,y,w,h` (crop in panel pixels); `-T seconds`; `-c` clipboard; `-x`, `-m`, `-r`,
  `-a`, `-t png`, `-D 1` accepted; one file operand; default name `Screenshot-<unix>.png` in the current folder.
- Refused by name: `-i -w -W -s -S -l` (selection modes need a shell overlay the control socket does not offer),
  `-C` (cursor), `-v -V -G -g -k` (video), `-b`, other formats, and `-M -P -B -p -u -U -J -o -d`.
- Gaps: `-R` is in pixels, not points; `-T` defaults to none, while the man page says the default is 5 (the man page
  wording for `-T` is ambiguous; we follow "no delay unless asked"); the default file name does not match the shell's
  `Screenshot-YYYYMMDD-HHMMSS.png` in Photos.
- Midget check: `screencapture /tmp/a.png` on Midget gives a PNG that matches `bifrost pc midget screen`; `-R` crops the
  same region; `-c` then paste into a text field shows the picture.

## say

- API: the macOS speech command line (`say [-v voice] [-r rate] [-o file] [-f file | text]`).
- BifrOSt path: `cmd/src/say.rs`, `cmd/src/bin/say.rs`. The desktop has no speech engine of its own; `say` runs the
  system `espeak-ng` (else `espeak`) as a package, feeding the text on standard input.
- Done: `-v`, `-r`, `-o`, `-f`, text operands, `--` before text; a clear error when no engine is installed.
- Refused by name: `-v ?` (voice list), `-a -n -i -e` and the file and data format options.
- Gaps: `-o` writes WAV, not AIFF; no voice list; the rate is passed through without range checks; no engine is in the
  Midget image yet.
- Midget check: install `espeak-ng` on Midget, then `say hello` is audible and `say -o hi.wav hello` writes a WAV.
  Without the package, `say` must print the install message and exit 1.

## caffeinate

- API: the macOS sleep-assertion command (`caffeinate [-disu] [-t timeout] [-w pid] [utility]`).
- BifrOSt path: `cmd/src/caffeinate.rs`, `cmd/src/bin/caffeinate.rs`. Holds logind block inhibitors through
  `systemd-inhibit`, the same mechanism as the shell's lid inhibitor (`bare::sleep::LidInhibitor`).
- Done: `-i` (`--what=idle`), `-s` (`--what=sleep`), `-t seconds`, `-w pid` (the hold ends when the process exits),
  a utility under the hold (its exit status is passed on), and the man page's default of idle when no flag is given.
- Refused by name: `-d` (display sleep) and `-u` (user activity): the shell keeps no display assertion yet. `-m`
  (disk idle): logind has no such block.
- Gaps: `-s` is held on battery as well as mains; the man page asks for mains only. `-t` and `-w` are ignored when a
  utility is given, as on macOS.
- Midget check: `caffeinate -t 60` then the Energy idle timeout (`pmset sleep 1`) does not suspend for the minute;
  `caffeinate make` runs the command and passes its status back.

## pmset

- API: the macOS power-settings command (`pmset -g`, `pmset [-a|-b|-c|-u] sleep <minutes>`).
- BifrOSt path: `cmd/src/pmset.rs`, `cmd/src/bin/pmset.rs`. Reads and writes the shell's `system_sleep_minutes` pref
  through `springboard::shell::prefs::Prefs`, the shell's own settings store, at the shell's `settings.ini`.
- Done: `-g` (prints the idle timeout, `never` for zero), `sleep <minutes>` (zero is never), the power source flags
  accepted (the desktop keeps one value), and root or user data directory chosen the way the shell picks it.
- Refused by name: `displaysleep` (no display-off timer setting exists yet), `sleepnow`, `displaysleepnow`, `touch`,
  `boot`, `schedule`, `repeat`, `relative`, `haltlevel`, `haltafter`, `haltremain`, and any other setting.
- Gaps: the shell reads the pref when it starts, so a change applies after the next start; `-g` prints only the idle
  timeout, not the assertion list.
- Midget check: `pmset sleep 2`, restart the shell, then the Settings > Energy page shows 2 minutes; `pmset -g` agrees.

## Shared code

- `cmd/src/control.rs` (new): the control socket client, moved out of `cmd/src/open.rs`. It reads the whole answer,
  so a raw frame comes back too. `open` now sends through it and keeps its public names (`open::port`, `open::send`).
  Its duplicate port and line tests moved to `control`.
- `cmd/Cargo.toml`: a new `springboard` path dependency, for the shell's PNG writer, settings store and data directory.
  Nothing in `springboard` depends on `bifrost-cmd`, so no cycle.

## Matrix entries (not closed; the lead decides)

These entries are `gap` today; this slice takes them to `partial` once the Midget checks pass:

- `path:/usr/sbin/screencapture` (`docs/parity-matrix/os/cli/s-s.json`)
- `path:/usr/bin/say` (`docs/parity-matrix/os/cli/s-s.json`)
- `path:/usr/bin/caffeinate` (`docs/parity-matrix/os/cli/a-c.json`)
- `path:/usr/bin/pmset` (`docs/parity-matrix/os/cli/p-r.json`)

## Not done (needs the shell or Midget)

- Interactive and window capture, and the cursor, need a selection overlay and a cursor source in the shell.
- A display and user-activity assertion (`caffeinate -d -u`) needs a display-sleep hold in the shell.
- Live `pmset` changes need a control request that makes the shell re-read `system_sleep_minutes`.
- A speech engine in the Midget image.
