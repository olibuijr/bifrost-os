# Software Update (parity slice)

Matrix entries this slice touches (rows are in `docs/parity-matrix/os/`; the lead updates them):

| Entry | Status before | BifrOSt path | Done in this slice |
|---|---|---|---|
| `softwareupdate` (CLI) | gap | `springboard/cmd/src/softwareupdate.rs`, `cmd/src/bin/softwareupdate.rs` | `-l`, `-i <label>`, `-a` (OS update), `--schedule` (prints the preference), `--schedule on\|off` (sets it) |
| `SoftwareUpdate` (prefPane) | partial | `springboard/src/shell/backup_page.rs`, `settings.rs` | not changed here |
| `SoftwareUpdateSettingsExtension` | partial | same as above | not changed here |
| `SoftwareUpdateNotificationManager` | gap | `springboard/src/shell/ota.rs` (`Notice`, `notices`) | existing banners (one per new OS version, one per app-update set); not changed here |

## Behaviour

- Background check: `shell/ota.rs` `Schedule` (30 s after start, on Wi-Fi connect, then every 15 minutes, never in
  Low Power Mode unless charging). Manual checks go straight to `Updater::check`.
- `--schedule off` writes `update-schedule-off` into the shell data folder (`ota::set_schedule`). `Ota::tick` skips
  the background schedule while the file exists (`ota::schedule_on`). The update agent (`springboard/updated`) and the
  shell's own `Ota` both read the same file, so the switch holds whichever process runs the checks. It survives restarts.
- `--schedule on` removes the file. The protocol command is `svc::update::Cmd::Schedule { automatic }` (tag 7).
  The agent answers commands in order, so the CLI sends `Schedule` then `Ping` and waits for the pong.
- Install: the OS update is staged by the agent (`springboard.new`, or pacman on a desktop). The restart prompt is the
  Software Update page, and `softwareupdate -i` / `-a` prints "Restart BifrOSt to finish installing."

## Reference evidence

Reference: the macOS 27 `softwareupdate(8)` man page at
`~/Projects/macos-ref/extract/ref27/usr/share/man/man8/softwareupdate.8` (read with `man -l`). It documents `-l`, `-i`
(with `-a`, `--os-only`, `--restart`), `--background` (a forced background scan) and `--schedule`, which "returns the
per-machine automatic (background) check preference". The man page does not document an on/off form of `--schedule`;
the on/off setter is a BifrOSt extension. Reading the preference (`--schedule` alone) reads the switch file, so it
does not need the agent. No reference files are copied into the repo. REA evidence ids: none recorded this round.

## Still a model or not done

- `softwareupdate --background` (a forced background scan) is not implemented; it would send the agent's `Check`.
- App updates have no install command on the agent protocol. `-i` and `-a` name only the OS update. App installs still
  happen through the store (automatic while the app is closed) and the App Store's Update button.
- Install progress for the pacman path is not reported (the agent's `fraction` covers the download only). The
  `InstallerProgress` row stays a gap.
- Not built or run here (the lead builds). Needs a Midget check: `softwareupdate --schedule off`, then confirm no
  background check runs after Wi-Fi reconnects and 15-minute intervals, and that `softwareupdate -l` still checks.
- The Settings page does not show the schedule switch yet.
