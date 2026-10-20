# BifrOSt PC: installer, boot, tools and endpoints

Source of truth for how a BifrOSt PC is installed, booted, updated and driven from the lab.
Verified against the code on 2026-10-13 (springboard 0.7.25). Update this file when a path,
port, unit or tool changes.

## 1. Who installs what

| Piece | Built by | Lives in | Delivered by |
|---|---|---|---|
| Live medium (ISO) | `tools/build-iso.sh` (`--lab`, `--debug`, `--release`) | `out/iso-*` and the iVentoy folder | Rebuild only for boot-path changes |
| Installer binary `bifrost-installer` | `springboard/src/bin/installer.rs` | live ISO `/usr/bin/bifrost-installer` | `tools/lab-push.sh` (lab hosts fetch it at boot) |
| Shell `springboard` | `springboard/src/bin/shell.rs` | installed by pacstrap from `springboard-shell` | `tools/pc-publish.py` into the lab repo |
| Boot loader `bootweave.efi` | built from `springboard/bootweave/efi` by `tools/build-efi-boot.sh` (the committed `packaging/arch/recovery/bootweave.efi` is a build output) | ESP `\EFI\BIFROST\BOOTx64.efi` and the `\EFI\BOOT\BOOTx64.efi` fallback | copied from the live medium by the installer |
| Early-userspace supervisor `bifrost-bootweave` | `springboard/bootweave/supervisor.rs` | `/usr/bin/bifrost-bootweave`, run by `springboard.service` | `springboard-shell` package |
| Window server `bifrost-wm`, chrome clients | `springboard/wm`, `springboard/chrome` | `/usr/bin/bifrost-wm` | `springboard-shell` package (built by `pc-publish.py`) |

The `springboard-shell` package is built with `packaging/arch/fast/PKGBUILD` and
`packaging/arch/package-files.sh`. It carries the system assets (see "System assets" below) at the
package-owned `/usr/share/springboard/system-assets`; the installer media (`/usr/share/bifrost/installer`, `video`)
come from the live medium.

### System assets (2026-10-20)

The shell's UI fonts, app icons and wallpapers used to live in `/usr/share/springboard/assets`, copied from the live
medium by the installer, so no package owned them and an update could never change them (a machine kept the icons it
was installed with, vendor-derived ones included). Now the update package owns them:

| Path | Owner | Content |
|---|---|---|
| `/usr/share/springboard/system-assets` | `springboard-shell` | the allowlisted assets (`packaging/assets-allowlist.txt`), staged by `tools/pc-publish.py` with `tools/stage-assets.py` |
| `/usr/share/springboard/assets` | nobody (live medium, older installs, host previews, tests) | the legacy location; read only when `system-assets` is empty or absent |
| `/usr/lib/springboard/prune-stale-assets`, `assets-allowlist.txt` | `springboard-shell` | the scriptlet's cleanup helper and the allowlist it checks against |

- **Readers.** `bifrost_core::system_assets` (`springboard/core/src/system_assets.rs`) is the one resolver: the
  `BIFROST_ASSETS` override, then `system-assets` when it holds files, then `assets`. The installer binary, the first-frame
  cache, the thumbnail canvas and the app sandbox's `BIFROST_SHELL_DIR` use it. The shell itself reads fonts, icons and
  wallpapers from its data dir `/var/lib/springboard`, which `/usr/lib/springboard/seed-data` fills at every start: with
  `system-assets` present it copies every file that differs whenever the set changed (a stamp in
  `/var/lib/springboard/.system-assets`) and makes `icons/` a mirror, so a retired icon disappears; without it the old
  copy-if-missing seeding from `assets` runs.
- **Existing installs.** pacman refuses to install a file that exists unowned, and the guard that runs an update is the
  one already installed, so the package uses a path no install has a file at: no conflict. `springboard.install`
  (`post_install`, `post_upgrade`) then runs `prune-stale-assets`: it removes files under `assets/` that no package owns
  (`pacman -Qo`) and that `system-assets` supersedes or the allowlist does not name, logs each one (package output and
  `/var/log/bifrost-assets.log`) and drops the directories that became empty. It never touches a file a package owns, is
  idempotent, and does nothing until `system-assets` has files. Tests: `python3 tools/test_asset_scripts.py`.
- **Guard.** `/usr/share/springboard/system-assets/*` is in the guard's `--overwrite` list, harmless today and ready for a
  file that ever exists unowned there. The installed guard runs the update, so a new entry first helps the update after next.
- **Installer.** It no longer copies `/usr/share/springboard/assets` from the medium: the package it installs brings
  `system-assets`. Only a package older than that (no `system-assets` in the target after `pacman`) makes it fall back to
  the medium's copy, pruned to the packed set (`live_assets_needed`). The first-frame cache is built from the target's own
  root (`system_assets::root_under("/mnt")`). The live medium itself keeps using `/usr/share/springboard/assets`.

## 2. Installer (`springboard/src/installer/`)

Entry: `bifrost-installer.service` (live medium, tty1). It runs the Rust
`/usr/bin/bifrost-installer` directly on DRM/KMS. The installed desktop uses
the separate `bifrost-wm` Wayland session.

Environment switches (`springboard/src/bin/installer.rs`, `plan.rs`):

| Variable | Effect |
|---|---|
| `BIFROST_AUTO=user:password:/dev/DISK` | Headless install with no UI. Erases DISK. Used by `midget-wipe.py`. |
| `BIFROST_TRACE=1` | Verbose step log to the journal (`plan.rs`). |
| `BIFROST_ASSETS` | Asset directory override. Default: `/usr/share/springboard/system-assets` when it has files, else `/usr/share/springboard/assets` (`bifrost_core::system_assets`). |
| `BIFROST_BRANDING`, `BIFROST_LAYOUT`, `BIFROST_NO_VIDEO`, `BIFROST_STATS` | UI and branding options. |
| `BIFROST_HWACCEL` | Hardware video selection (`video.rs`). |
| `BIFROST_DISK_FIXTURE` | Test fixture for the disk list (`disks.rs`). Never set on hardware. |

Install order (`plan::install`):

1. `stage_live_files`: copy `/usr/share/bifrost/bootweave.efi`, `/usr/share/springboard/assets` (kept only as the
   fallback for an old package) and `/usr/share/bifrost` into RAM (`/run/bifrost-stage`). A dropped network medium cannot break an install
   that has started. The stage copy happens before anything is erased.
2. Partition (`disks::partition_commands`): ESP (512 MiB vfat) and one btrfs root labelled `BifrOSt`.
3. Mount subvolumes (`BTRFS_LAYOUT`): `@system /`, `@home /home`, `@state /var/lib/springboard`,
   `@recovery /var/lib/bifrost-recovery`, `@cache /var/cache`, `@logs /var/log`.
4. `pacstrap -K /mnt` with `BASE_PACKAGES` + `runtime-packages.txt` + GPU packages from `lspci`.
5. Configure: fstab, hostname, locale, keyboard (`/etc/bifrost/keyboard.env`), os-release, sshd drop-in.
   A lab medium (`/etc/bifrost/lab.token` present) copies the lab root key and sets
   `PermitRootLogin prohibit-password`.
6. Update channel: `/etc/pacman.conf` gets `[bifrost]`. Lab: `http://192.168.1.12:8079/bifrost/$arch` (Titan home LAN, served by the `bifrost-repo` unit).
   Release: `https://bifrost.olibuijr.com/arch/$arch`. The lab server also serves the package mirror on :8079, synced by `bifrost pc publish` (`tools/pc_mirror.py`, `docs/installer.md`).
7. Copy the installer media (`/usr/share/bifrost/installer`, `video`; not the assets) from the stage, then
   `pacman -Sy --noconfirm springboard-shell`, which brings `system-assets`. Only when the target has none (a package
   older than that) is the medium's `assets` copied afterwards and pruned.
8. Look: the look chosen in the installer goes into `/var/lib/springboard/settings.ini`.
9. Browser: `springboard --install-store-app is.olibuijr.browser.desktop` in the chroot.
10. Account: `useradd -m -G wheel,audio,video,input,storage`, sudo for wheel, root locked,
    `/var/lib/springboard/account` (read by `session` for the local account name).
11. Boot: `bootweave.efi` → `\EFI\BIFROST\BOOTx64.efi` and `\EFI\BOOT\BOOTx64.efi`, `boot.cfg` and `cmdline.txt` (below),
    `efibootmgr -c -L BifrOSt -l \EFI\BIFROST\BOOTx64.efi`, mkinitcpio preset `linux-rt` (`linux` when the fallback below
    applied), then `bifrost-update-guard uki` builds `EFI/BIFROST/bifrost.efi` and the first-frame caches are written.
12. Services (chroot): mask `getty@tty1`; enable `bifrost-update-guard`, `springboard`, `NetworkManager`, `sshd`, `seatd`,
    `power-profiles-daemon`, `bifrost-irq-priority`, `bifrost-mcp`; default target `graphical`. Runtime and reboot
    watchdogs are set to 60 s and 120 s.
13. Browser store app, then the summary screen. Progress stages: `Preparing the disk`,
    `Installing the system`, `Setting up your account and apps`, `Finishing`.

Resolved (2026-10-20): the assets are in the package (`system-assets`). Before that (found 2026-10-08) an install
whose stage lacked `/usr/share/springboard/assets` started a shell that exited with `No such file or directory`.

## 3. Boot chain (installed PC)

1. UEFI runs `\EFI\BIFROST\BOOTx64.efi` (`bootweave.efi`; `\EFI\BOOT\BOOTx64.efi` is the fallback copy). The NVRAM entry is `BifrOSt`.
2. `boot.cfg` (`plan::cmdline_txt`):
   `initrd=\initramfs-linux-rt.img root=UUID=<root> rootflags=subvol=@system rw quiet loglevel=3 rd.udev.log_level=3 udev.log_level=3 rd.systemd.show_status=false systemd.show_status=false systemd.log_target=journal rd.luks=0 rd.lvm=0 rd.md=0 rd.dm=0 vt.global_cursor_default=0 fbcon=map:9 systemd.tty.rows.console= i915.enable_fbc=0 i915.enable_psr=0 panic=10 threadirqs`
   (the i915 flags stop Skylake framebuffer-compression stripes; `fbcon=map:9` keeps the console off the display, so recovery
   uses ssh or serial; `threadirqs` is for the audio IRQ threads; an audio core adds `isolcpus`, `nohz_full`, `rcu_nocbs`). When the UKI exists the loader starts it directly.
3. initramfs (mkinitcpio `linux-rt` preset) mounts `@system` and switches root.
   The installed kernel is `linux-rt` (Arch PREEMPT_RT, default since 2026-10-12, `docs/audio-latency.md` "Kernel"):
   `/boot/vmlinuz-linux-rt`, `/boot/initramfs-linux-rt.img`. Machines with an NVIDIA GPU get the stock `linux` (`kernel_for`: `nvidia-open` modules match only it, and NVIDIA modules
   refuse PREEMPT_RT). If `pacman -Sp linux-rt` fails (the package source lacks it) the installer logs it and uses `linux`, writing the `linux` preset and
   `initramfs-linux.img`, so an install never fails over the kernel. The loader derives the kernel from the `initrd=`
   name (`initramfs-linux-rt.img` -> `vmlinuz-linux-rt`, anything else -> `vmlinuz-linux`), and the update guard
   (`packaging/arch/recovery/system.rs`) picks the kernel files, preset and UKI inputs by which image is in `/boot`,
   so older stock-kernel installs keep working. The live ISO keeps the stock `linux` kernel.
4. No Plymouth. The EFI frame stays in simpledrm; the live medium's `bifrost-installer.service` redraws it and owns the card directly.
5. `springboard.service` (no default dependencies; after udev and `seatd`; `Conflicts=getty@tty1`) runs `/usr/bin/bifrost-bootweave run`,
   which supervises `/usr/lib/bifrost/session` (and shows the start-failed screen after three quick failures):
   - reads the account from `/var/lib/springboard/account`, cleans `/run/bifrost/user/{wayland-*,session.json,shell-ready}`;
   - starts `seatd`; starts `bifrost-wm` as the account (the only window server; after three quick exits it restarts in safe mode: GLES, no direct apps, no chrome clients, then the recovery flow); the environment words (`BIFROST_DIRECT`, `BIFROST_WM_RENDERER`, `BIFROST_CHROME`) come from `wayland/session-policy`;
   - waits up to 10 s for `wayland-0` and `session.json`; then starts `/usr/bin/springboard`.
6. The shell writes `shell-ready` after its first committed Wayland frame (`springboard/src/bare/wayland.rs`,
   `mark_first_frame`). `bifrost-wm` holds the boot frame until the desktop is ready and then fades once (`docs/boot-architecture.md`).
   `bifrost-desktop-ready.service` starts `bifrost-after-desktop.target` after the `desktop-ready` mark (6 s ceiling).

Boot timing: the boot marks (`bootweave-start`, `compositor-ready`, `desktop-first-frame`, `desktop-ready`, `desktop-first-ui`,
`desktop-faded`, ...) go to `/run/bifrost/boot-marks`; `tools/boot-bench.py` reads them. The older `/var/log/bifrost-boot-timing.log`
was for the removed Plymouth handoff. Measured on Midget (0.4.37): firmware 9.4 s, loader 0.6 s, kernel 0.9 s, initrd 3.3 s,
userspace 5.0 s, `graphical.target` at 4.98 s. The tty/serial/tpm device units at the top of `systemd-analyze blame`
(~4 s) are not on the critical chain to `graphical.target` (`NetworkManager` -> `systemd-user-sessions` ->
`springboard.service`), so masking them gains nothing. The window server picks the mode itself; it needs no mode config.

Crash-loop guard: the shell (`springboard/src/bare/takeover.rs`) counts starts that do not stay up for 60 s in
`/var/lib/springboard/boot_attempts`. After three it exits 70 and stays down. The session clears the counter whenever the
installed `springboard-shell` version changes (`packaging/arch/wayland/session`, `shell-version` stamp), and the installer
clears it at the end of install. Clear it by hand with `rm /var/lib/springboard/boot_attempts; systemctl restart springboard`.

Restart policy: `springboard.service` restarts on failure. Exit status 70 means the shell crashed on start.
Check `journalctl -b -u springboard` and run the shell by hand as the account to see its stderr.

Update path (after install): Settings › Software Update, or `bifrost pc midget update`, runs
`pacman -Sy springboard-shell` from `[bifrost]`. `bifrost-update-guard` (`packaging/arch/recovery/guard.rs`)
stages a Btrfs generation, reboots for probation and rolls back if probation fails. Its phases:
`prepared`, `system-prepared`, `system-probation`, `probation`, `accepted`, `restoring`, `rolled-back`.
CLI: `bifrost-update-guard status|prepare|install|watch|rollback`.

## 4. Live-lab hosts (lab ISO only)

- `bifrost-lab.service` (lab builds, `/etc/bifrost/lab.token`): HTTP API on `0.0.0.0:8651`.
  Endpoints (`packaging/archiso/airootfs/usr/local/bin/bifrost-lab`): `/lab/info`, `/lab/log`,
  `/lab/screenshot` (`?fb=1` for the framebuffer), `/lab/key`, `/lab/type`, `/lab/click`, `/lab/move`,
  `/lab/exec`. Requires the lab token.
- `bifrost-lab-sync` (`ExecStartPre` of the installer): fetches the newest installer and shell from
  `http://10.99.0.1:8078` (sync dir `out/lab/sync`, manifest in `tools/lab-push.sh`) at boot.
- `bifrost-debug-log` (debug ISO): streams the journal to `tools/lab-log-sink.py`.

## 5. Control socket (running shell)

`springboard/src/bare/control.rs`, Unix socket `/run/bifrost/shell-control.sock` (mode 0600, owner the installed account; served to root and that account by `SO_PEERCRED`; override `SPRINGBOARD_CONTROL_SOCKET`). There is no TCP listener. Line commands:
`state`, `shot [N]` (1-8), `tap X Y`, `swipe X0 Y0 X1 Y1 [MS]` (alias `drag`), `key ...`, `trace on|off`.
Coordinates are original panel pixels. `tools/midget.py` sends each line over SSH as root and speaks `AF_UNIX` to that socket on Midget (`cmd_control`, `control_script`); nothing is exposed on the lab network.

## 6. Endpoints

| Address | Service | Owner |
|---|---|---|
| `10.99.0.1:8078` | lab sync (installer and shell for lab hosts) | `tools/lab-push.sh` |
| `192.168.1.12:8079/bifrost/x86_64` | lab pacman repo (`springboard-shell`) | `tools/pc-publish.py` |
| `192.168.1.12:8079/mirror/$repo/os/$arch` | package mirror (core, extra) for guarded updates | `tools/pc_mirror.py`, run by `pc-publish.py` |
| `10.99.0.1` `eno2` | iVentoy PXE/netboot (ISO folder) | titan |
| `192.168.20.140:22` | Midget sshd (lab key on lab installs only) | installed system |
| `192.168.20.140:8651` | `bifrost-lab` API (lab ISO only) | live medium |
| `https://bifrost.olibuijr.com/arch/$arch` | release pacman repo; live static files under `bifrost-site/site/arch/$arch` | `tools/pc-publish.py` |
| `192.168.1.12` | Titan (lab host, Midget control scripts run here) | titan |

### Package signing

The `[bifrost]` repository is signed with a release key; the root-run `bifrost-update-guard` installs whatever it serves, so the
signature is the only thing between a tampered mirror and root on every machine. Two phases:

`tools/pc-publish.py` signs every package (`makepkg --sign`) and the database (`repo-add --sign`) with the key named by
`BIFROST_REPO_SIGN_KEY` (the fingerprint is kept in `out/lab/repo-sign-key.fpr`) in the GnuPG home `BIFROST_REPO_GNUPGHOME`
(default `out/lab/gnupg`, never in git); the `.sig` files are published next to the files. The public key
(`packaging/arch/keys/bifrost-repo.gpg`, see the README there) is shipped by `springboard-shell` at `/usr/share/bifrost/keys/`
and imported and locally signed into pacman's keyring by its scriptlet (`bifrost_repo_key` in `springboard.install`); the
installer imports it from the live medium (`tools/build-iso.sh` copies it there) before the first sync. Installs write
`SigLevel = Required DatabaseRequired` for `[bifrost]` (`BIFROST_REPO_SIG_LEVEL` in `installer/plan.rs`): an unsigned package or
database, or one signed by a key the machine does not trust, is refused. There was no unsigned installed base to migrate (the
only installed machine, Midget, had the key imported and its `pacman.conf` switched by hand on 2026-10-20).

## 7. CLI tools (`tools/`)

| Tool | Use | Notes |
|---|---|---|
| `bifrost pc midget status|state|screen|tap|swipe|key|trace|update|logs|bootnext|reboot|poweroff|wake|bios|me|amt|hw|efi|ssh` | Day-to-day control of Midget | Wraps `tools/midget.py`. Input needs explicit approval per test. |
| `tools/midget.py wipe --disk DEV --confirm "WIPE 10.99.0.200 DEV" [--dry-run]` | Wipe and reinstall Midget | Calls `tools/midget-wipe.py`. |
| `tools/midget-wipe.py` | Guarded erase and reinstall | Preflight: MAC `00:23:24:dc:c8:40`, exactly one erasable disk equal to `--disk`, root must not be that disk, exact token. Steps: PXE to live medium, `BIFROST_AUTO` install, BootNext to disk, verify ssh and `springboard`. Logs in `out/lab/wipes/<stamp>/`. |
| `tools/build-iso.sh [--lab|--debug|--release] [--profile fast|release] [--deploy]` | Build the live medium | Needs sudo. `--release` defaults to the full Cargo release profile; an explicit `--profile fast` selects the optimized incremental profile. Requires a current local package mirror and default app packages; missing offline bundle inputs stop the build. Only allowlisted assets reach the image (`stage-assets.py`), and the licence guard reads the finished airootfs, the bundled shell package and the default apps before `mkarchiso`. |
| `tools/release-vm-test.py --iso PATH` | Install an unchanged release ISO in a disposable 24 GiB UEFI VM and verify the installed desktop | Creates a unique run directory and disk. QEMU blocks outbound guest traffic and exposes only a loopback-forwarded diagnostic SSH port. The runner verifies release hardening before adding a VM-only key, drives the real UI, and records reports/screenshots under `out/vm-runs/`. The ISO must contain all offline packages and apps. |
| `tools/lab-push.sh` | Push installer and shell to lab hosts | No ISO rebuild. |
| `tools/pc-publish.py [--release] [--skip-build] [--skip-mirror-sync]` | Build and publish `springboard-shell`, then sync and check the :8079 mirror | Fast profile. `bifrost pc publish` is the same. Version is `Cargo.toml` plus a timestamp. Builds `bifrost-wm`, the chrome clients and `bifrost-log` too. The package goes through the licence guard before `repo-add`; a failing one is deleted and the publish stops. |
| `tools/pc_mirror.py [--sync] [--check]` | The :8079 package mirror alone | `test_pc_mirror.py` has the tests. |
| `tools/lab-up.sh [down]` | Lab NIC, nft NAT table `bifrost_nat` for 10.99.0.0/24, iVentoy | Run after a titan reboot. |
| `tools/pc-app.py`, `pc-app-smoke.py` | Desktop app build and publish | See `docs/bifrost-pc-workflow.md`. `pc-app.py` runs the licence guard on the `.aap` and deletes a failing one. |
| `tools/check-package-licenses.py [--allow FILE] TARGET...` | Licence guard for a `.pkg.tar.zst`, an `.aap` or a directory tree | Exit 1 and the offending paths when a path is under `usr/share/bifrost/ref` or a font names no open licence (OpenType name table IDs 13, 14, 0, every face of a collection). Excuses go in `tools/check-package-licenses-allow.txt` (path or family, with a reason). Run by `pc-publish.py`, `pc-app.py`, `build-iso.sh` and the local `PKGBUILD`; `test_check_package_licenses.py` has the tests. See `docs/bifrost-pc-workflow.md`, "Package licences". |
| `tools/stage-assets.py SRC DEST [--replace] [--list]` | Copy `springboard/assets` into a package or ISO tree | Copies only the files `packaging/assets-allowlist.txt` names, so a stray vendor file never ships; links are skipped. Used by the local `PKGBUILD`, `build-iso.sh`, `lab-push.sh` and `arch-vm.sh`; `test_stage_assets.py` has the tests. |
| `tools/vm-install.sh`, `vm-cycle.py` | VM install loop | `vm-cycle` wipes by simulated keystrokes (fragile). |
| `tools/iventoy-prune.sh` | Prune old ISOs in iVentoy | Keeps the newest 2 and anything under 4 h old. Never delete an ISO a machine booted from. |
| `tools/test_midget.py`, `tools/test_midget_wipe.py` | Unit tests for the scripts above | Run without touching hardware. |

## 8. Procedures

**Known failure chain (2026-10-08):** a shell start that fails for any reason (missing assets, window-server swapchain
rejected on DP-2, a stale DRM holder) counts toward the crash guard. Three of them latch the desktop off until the counter
is cleared. Diagnose with `journalctl -b -u springboard`, then run the shell as root against a live `bifrost-wm`, since the
session starts it as root and its data lives in `/var/lib/springboard`. Running it as the account reads the wrong data dir.
Intel Skylake stripes on the splash came from i915 framebuffer compression. The installed kernel command line sets
`i915.enable_fbc=0 i915.enable_psr=0`.

**Ship a shell or desktop change (no ISO):** bump `springboard/Cargo.toml` and `CHANGELOG.md`,
`tools/pc-publish.py`, then Midget picks it up through Software Update.

**Ship an installer change:** `tools/lab-push.sh`. The next netboot of a lab host runs it.

**Change boot-time files (loader, cmdline, initramfs):** the loader and cmdline come from the live medium. That needs `tools/build-iso.sh --lab`
and a new ISO in iVentoy. Confirm with the user first.

**Wipe Midget:** `tools/midget.py wipe --disk /dev/nvme0n1 --confirm "WIPE 10.99.0.200 /dev/nvme0n1" --dry-run`,
then the same command without `--dry-run`. Run the wipe as a foreground job, or background it with
`setsid`. Plain `nohup` dies when the calling shell times out, which happened on 2026-10-08.
Review `out/lab/wipes/<stamp>/log.txt` and `report.json`.

**Diagnose a Midget that does not show the desktop:** `pacman -Q springboard-shell`, `systemctl status springboard`,
`journalctl -b -u springboard`, then run `/usr/bin/springboard` by hand as the account with
`XDG_RUNTIME_DIR=/run/bifrost/user WAYLAND_DISPLAY=wayland-0` (`bifrost-wm` must be up).
