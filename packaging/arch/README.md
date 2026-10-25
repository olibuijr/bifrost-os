# springboard-shell (Arch package)

Installs the SpringBoard shell as the sole UI of an x86_64 Arch machine (desktop profile,
`SPRINGBOARD_PLATFORM=desktop`). Local only; nothing is published.

| Path | Content |
|---|---|
| `/usr/bin/springboard` | the `shell` binary (`cargo build --release --bin shell`) |
| `/usr/share/springboard/system-assets` | the allowlisted part of the packed assets (`packaging/assets-allowlist.txt`: wallpaper, open-licence fonts, BifrOSt's own icons); owned by the package, so updates change it |
| `/usr/share/springboard/assets` | owned by nobody: the live medium's and older installs' copy; `prune-stale-assets` empties it of what the package supersedes |
| `/var/lib/springboard` | data dir (root's desktop-profile default); `seed-data` fills it from `system-assets` (else `assets`) at every start (`.seeded` marker) |
| `/usr/lib/systemd/system/springboard.service` | runs the shell on tty1 |
| `/usr/lib/springboard/seed-data` | the seeding script (copies changed assets, mirrors `icons/`) |
| `/usr/lib/bifrost/services/<id>/` | the system services `bifrostd` supervises: `is.olibuijr.audio` (`audiod`), `is.olibuijr.net` (`netd`), `is.olibuijr.update` (`updated`), each with its `manifest.toml` (from `springboard/system/<dir>`) and executable; a package-owned path, so an update changes it with no conflict (see `springboard/docs/services.md`, "System services ship with the OS") |
| `/usr/share/stardict/wordnet.{ifo,idx,dict}` | the system English dictionary the Dictionary app and Look Up read: WordNet 3.0 converted to StarDict by `tools/build-wordnet-stardict.py` (pinned tarball and SHA-256 in `dictionaries/wordnet.json`, cached in `out/upstream/`); licence: WordNet 3.0 licence (Princeton University, redistribution permitted with the notice), notice at `/usr/share/licenses/springboard-shell/LICENSE-WordNet` |
| `/usr/lib/springboard/prune-stale-assets`, `assets-allowlist.txt` | the scriptlet's cleanup of unowned files under `assets/` (see below) |

## Build

```
cd packaging/arch
makepkg -f            # or: makepkg --nobuild --nodeps   for a source/prepare check only
```

`springboard/Cargo.toml` path-depends on two sibling repos by absolute path
(`/home/olibuijr/Projects/{iOSTerm,ipad-deildu-rust}`). `prepare()` copies the checkout and those
repos (without `target/` and `.git`) into `$srcdir/tree` and rewrites the paths there, so the build tree is
self-contained and the checkout is untouched. Point it elsewhere with `SPRINGBOARD_DEPS=<parent dir of the
two repos>`, `SPRINGBOARD_REPO=<bifrost-os checkout>`, `SPRINGBOARD_ASSETS=<assets dir>`.
Crates are not vendored: cargo runs `--offline` against the local registry cache (`CARGO_HOME`), so warm it
first with `cargo fetch` in `springboard/` (or pass `SPRINGBOARD_CARGO_FLAGS=` to allow network).
Use `BUILDDIR=` to keep `src/`/`pkg/` out of the checkout.
The assets directory is untracked (about 244 MB, produced by `tools/pack.py ../ipad-ref assets W H`); without it the
package still builds but ships an empty system-assets dir.

## What a package may carry

Only BifrOSt's own files and openly licensed ones. `package()` copies the assets through `tools/stage-assets.py`, which
takes only the files named in `packaging/assets-allowlist.txt` (a vendor font or an icon taken from another system that
sits in the assets directory is skipped), nothing is read from `bifrost-ref`, and the finished `$pkgdir` must pass
`tools/check-package-licenses.py` (no path under `/usr/share/bifrost/ref`, every font names an open licence in its name
table). `tools/pc-publish.py`, `tools/pc-app.py` and `tools/build-iso.sh` run the same guard on what they build; see
`docs/bifrost-pc-workflow.md`, "Package licences". The fast update package (`fast/PKGBUILD`) carries the same staged
assets (`tools/pc-publish.py` passes them as `PC_SYSTEM_ASSETS`).

## System assets and the scriptlet

Both packages install the assets at `/usr/share/springboard/system-assets`, never at `/usr/share/springboard/assets`:
pacman refuses a file that exists on disk unowned, and every installed machine has an unowned `assets/`. In
`springboard.install`, `post_install` and `post_upgrade` run `/usr/lib/springboard/prune-stale-assets` before anything
else: it removes the files under `assets/` that no package owns (`pacman -Qo`) and that `system-assets` supersedes or
`assets-allowlist.txt` does not name, logs each removal (package output and `/var/log/bifrost-assets.log`), removes the
directories that became empty, never touches a file a package owns, and does nothing until `system-assets` has files. The
update guard's `--overwrite` list includes `/usr/share/springboard/system-assets/*`. Details and the reader order:
`docs/installer-and-boot.md`, "System assets". Tests: `python3 tools/test_asset_scripts.py`.

## Why root on tty1

The unit runs as root with `Conflicts=getty@tty1.service`. DRM master on `/dev/dri/card*` and the evdev nodes
in `/dev/input` then work without seatd/logind session plumbing, extra groups or udev rules, which is the
simplest thing that satisfies the shell's direct KMS + evdev use. It also makes `/var/lib/springboard` the
data dir (the desktop profile picks that path for root). A dedicated user would need `video`, `input`,
`audio` membership plus `seatd` (or logind with a PAM session on tty1) to obtain DRM master; do that later if
privilege separation matters.

## Enable

`systemctl enable springboard.service && systemctl set-default graphical.target`, then reboot. The unit
restarts on failure (2 s). `tools/arch-vm.sh` tests the same unit in a QEMU guest (`docs/arch-vm.md`).

## Updates (Settings > General > Software Update)

The page shows the installed package version
(`pacman -Q springboard-shell`) and checks read-only: `checkupdates` (pacman-contrib) when installed, else
`pacman -Qu` plus `pacman -Si`. "Download and Install" is the confirm; it runs
`/usr/bin/bifrost-update-guard install` (a managed transaction with shell rollback), then "Install Now"
restarts the shell into the new binary. As root (the shipped unit) it runs directly. A shell running as a
normal user goes through `sudo -n`, so allow exactly that command, in `/etc/sudoers.d/springboard`:

```
springboard ALL=(root) NOPASSWD: /usr/bin/bifrost-update-guard install
springboard ALL=(root) NOPASSWD: /usr/bin/bifrost-update-guard rollback-start
```

(replace `springboard` with the user the shell runs as).

## Recovery

- The unit restarts the shell on failure. The shell counts starts that never stayed up 60 s; after 3 in a row
  it exits with status 70 without starting, so a StartLimit in the unit (`StartLimitBurst=5`,
  `StartLimitIntervalSec=120`, `OnFailure=getty@tty1.service`) ends in a getty on tty1. The counter
  (`boot_attempts` in the data dir) goes stale after 10 minutes, so a manual `systemctl restart` after a fix works.
- Ctrl + Alt + Backspace held for 3 s on any keyboard exits the shell with status 71; systemd starts it again.
- Suspend goes through logind (`systemctl suspend`, refused while another program holds a sleep block
  inhibitor). The lid switch is logind's; `lid_close=ignore` in settings.ini makes the shell hold a
  `handle-lid-switch` block inhibitor.
- Time: systemd-timesyncd keeps the clock (the shell's own SNTP client is off); the zone is `/etc/localtime`
  and a zone picked in Settings is applied with `timedatectl set-timezone`.

New installs use Btrfs; see [system recovery](../../docs/system-recovery.md) for
subvolumes, shell probation and the current whole-OS recovery limitation.
