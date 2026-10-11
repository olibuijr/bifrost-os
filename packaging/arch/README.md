# springboard-shell (Arch package)

Installs the SpringBoard shell as the sole UI of an x86_64 Arch machine (desktop profile,
`SPRINGBOARD_PLATFORM=desktop`). Local only; nothing is published.

| Path | Content |
|---|---|
| `/usr/bin/springboard` | the `shell` binary (`cargo build --release --bin shell`) |
| `/usr/share/springboard/assets` | packed assets (`springboard/tools/pack.py`: wallpaper, fonts, icons, sounds) |
| `/var/lib/springboard` | data dir (root's desktop-profile default); seeded from the assets on first start (`.seeded` marker) |
| `/usr/lib/systemd/system/springboard.service` | runs the shell on tty1 |
| `/usr/lib/springboard/seed-data` | the first-start seeding script |

## Build

```
cd packaging/arch
makepkg -f            # or: makepkg --nobuild --nodeps   for a source/prepare check only
```

`springboard/Cargo.toml` path-depends on three sibling repos by absolute path
(`/home/olibuijr/Projects/{iOSTerm,wow-private,ipad-deildu-rust}`). `prepare()` copies the checkout and those
repos (without `target/` and `.git`) into `$srcdir/tree` and rewrites the paths there, so the build tree is
self-contained and the checkout is untouched. Point it elsewhere with `SPRINGBOARD_DEPS=<parent dir of the
three repos>`, `SPRINGBOARD_REPO=<tab-a8 checkout>`, `SPRINGBOARD_ASSETS=<assets dir>`.
Crates are not vendored: cargo runs `--offline` against the local registry cache (`CARGO_HOME`), so warm it
first with `cargo fetch` in `springboard/` (or pass `SPRINGBOARD_CARGO_FLAGS=` to allow network).
Use `BUILDDIR=` to keep `src/`/`pkg/` out of the checkout.
The assets directory is untracked (about 244 MB, produced by `tools/pack.py ../ipad-ref assets W H`); without it the
package still builds but ships an empty assets dir.

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
