# CLI: mdfind, mdutil, softwareupdate, launchctl, diskutil (slice cli-search-update-services)

Reference: the shipped man pages `mdfind.1`, `mdutil.1` and `launchctl.1` under
`~/Projects/macos-ref/extract/ref27/usr/share/man/man1/`. The flag list follows the slice brief. No REA evidence was
gathered in this pass, and the `softwareupdate` and `diskutil` man pages were not read, so their options are the
ones the brief names.

Matrix entries (not edited here, the lead closes them): `path:/usr/bin/mdfind` and `path:/usr/bin/mdutil`
(cli/m-o.json), `path:/bin/launchctl` (cli/j-l.json), `path:/usr/sbin/softwareupdate` and `path:/usr/sbin/diskutil`
(cli/s-s.json, cli/d-f.json).

## Code

| Tool | Binary | Logic | What works | Gap |
|---|---|---|---|---|
| mdfind | `cmd/src/bin/mdfind.rs` | `cmd/src/mdfind.rs` | `-onlyin`, `-count`, plain-text name match over a folder walk (hidden entries and symlinks skipped, depth 16) | `-name` is accepted but is the only match; no content or metadata index; `-live` refused; `kMDItem` queries are plain text |
| mdutil | `cmd/src/bin/mdutil.rs` | `cmd/src/mdutil.rs` | `-s`, `-i on/off`, `-E` on per-volume settings under `$XDG_STATE_HOME/bifrost/index/<volume>/` | the desktop quick search does not read these settings, so `-i off` does not stop its index yet; `-E` removes only the command line's store |
| softwareupdate | `cmd/src/bin/softwareupdate.rs` | `cmd/src/softwareupdate.rs` | `-l` (sends Check, reads the agent state), `-i <label>` and `-a` (download and stage the OS offer) over the update agent socket (`bifrost-proto` `svc::update`) | `--schedule` refused (the agent protocol has no schedule command); app updates have no install command on the protocol |
| launchctl | `cmd/src/bin/launchctl.rs` | `cmd/src/launchctl.rs` | `list`, `print`, `start`, `stop`, `enable`, `disable`, `kickstart [-k]` as `systemctl --user` on the label's `.service` unit | no `gui/<uid>` domains, no `load`/`unload` plists, PID column is `-` in `list`, no last-exit status |
| diskutil | `cmd/src/bin/diskutil.rs` | `cmd/src/diskutil.rs` | `list`, `info`, `mount`, `unmount`, `eject`; reads `/sys/block` and `/proc/mounts`; changes through `bifrost_core::disk_session` and `disk_commands` with the same claims and approval gates | no erase, partition or rename; no file system or label column (the app reads udev) |

## Adopt and delete

- `Refusal::sentence()` moved into `springboard/core/src/disk_commands.rs`. The diskutil app's `refusal_text` is
  deleted and `window.rs` calls `refusal.sentence()`. The app goes to 0.1.1 (manifest, Cargo.toml, CHANGELOG).
- The `diskutil` tool keeps its own `/sys/block` reader and eject plan. These duplicate `apps/diskutil-desktop/src/sysfs.rs`
  and `actions.rs::plan` and should move to `bifrost-core` in a follow-up so both read one model.
- The desktop quick search (`src/shell/quicksearch.rs`, `spotindex`) and the update agent's own settings are not
  wired to the command line tools yet, which is why `mdutil -i` and `softwareupdate --schedule` are limited.

## Still needs checking on Midget

- `softwareupdate -l` and `-a` against a running `updated` (the socket path follows `BIFROST_SERVICE_SOCK` or the
  shell data folder).
- `diskutil eject` on a real USB stick (the power-off command comes from `disk_commands`).
- `launchctl` against the user's systemd services, with a unit that is not in the list.

## Shared-file edits for the lead

- `springboard/cmd/Cargo.toml`: new dependency `bifrost-proto`.
- `springboard/Cargo.lock`: the `bifrost-cmd` entry lists `bifrost-proto`.
- `springboard/cmd/src/lib.rs`: five `pub mod` lines.
- `springboard/core/src/disk_commands.rs`: new `impl Refusal { fn sentence }` block (additive).
