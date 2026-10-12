# BifrOSt PC: installer, boot, tools and endpoints

Source of truth for how a BifrOSt PC is installed, booted, updated and driven from the lab.
Verified against the code on 2026-10-10 (springboard 0.4.85). Update this file when a path,
port, unit or tool changes.

## 1. Who installs what

| Piece | Built by | Lives in | Delivered by |
|---|---|---|---|
| Live medium (ISO) | `tools/build-iso.sh` (`--lab`, `--debug`, `--release`) | `out/iso-*` and the iVentoy folder | Rebuild only for boot-path changes |
| Installer binary `bifrost-installer` | `springboard/src/bin/installer.rs` | live ISO `/usr/bin/bifrost-installer` | `tools/lab-push.sh` (lab hosts fetch it at boot) |
| Shell `springboard` | `springboard/src/bin/shell.rs` | installed by pacstrap from `springboard-shell` | `tools/pc-publish.py` into the lab repo |
| Boot loader `bootweave.efi` | `packaging/arch/recovery/bootweave.efi` | ESP `\EFI\BOOT\BOOTx64.efi` | copied from the live medium by the installer |
| Plymouth theme | `packaging/archiso/airootfs/usr/share/plymouth/themes/bifrost` | installed `/usr/share/plymouth/themes/bifrost` | `springboard-shell` package |
| Handoff unit and script | `packaging/arch/bifrost-splash-handoff.service`, `packaging/arch/wayland/splash-handoff` | installed `/usr/lib/systemd/system`, `/usr/lib/bifrost` | `springboard-shell` package |

The `springboard-shell` package is built with `packaging/arch/fast/PKGBUILD` and
`packaging/arch/package-files.sh`. It has no assets: `/usr/share/springboard/assets` and the
installer media (`/usr/share/bifrost/installer`, `video`) come from the live medium.

## 2. Installer (`springboard/src/installer/`)

Entry: `bifrost-installer.service` (live medium, tty1). It runs the Rust
`/usr/bin/bifrost-installer` directly on DRM/KMS. The installed desktop uses
the separate Wayland session until the native BifrOSt compositor is complete.

Environment switches (`springboard/src/bin/installer.rs`, `plan.rs`):

| Variable | Effect |
|---|---|
| `BIFROST_AUTO=user:password:/dev/DISK` | Headless install with no UI. Erases DISK. Used by `midget-wipe.py`. |
| `BIFROST_TRACE=1` | Verbose step log to the journal (`plan.rs`). |
| `BIFROST_ASSETS` | Asset directory, default `/usr/share/springboard/assets`. |
| `BIFROST_BRANDING`, `BIFROST_LAYOUT`, `BIFROST_NO_VIDEO`, `BIFROST_STATS` | UI and branding options. |
| `BIFROST_HWACCEL` | Hardware video selection (`video.rs`). |
| `BIFROST_DISK_FIXTURE` | Test fixture for the disk list (`disks.rs`). Never set on hardware. |

Install order (`plan::install`):

1. `stage_live_files`: copy `/usr/share/bifrost/bootweave.efi`, `/usr/share/springboard/assets` and
   `/usr/share/bifrost` into RAM (`/run/bifrost-stage`). A dropped network medium cannot break an install
   that has started. The stage copy happens before anything is erased.
2. Partition (`disks::partition_commands`): ESP (512 MiB vfat) and one btrfs root labelled `BifrOSt`.
3. Mount subvolumes (`BTRFS_LAYOUT`): `@system /`, `@home /home`, `@state /var/lib/springboard`,
   `@recovery /var/lib/bifrost-recovery`, `@cache /var/cache`, `@logs /var/log`.
4. `pacstrap -K /mnt` with `BASE_PACKAGES` + `runtime-packages.txt` + GPU packages from `lspci`.
5. Configure: fstab, hostname, locale, keyboard (`/etc/bifrost/keyboard.env`), os-release, sshd drop-in.
   A lab medium (`/etc/bifrost/lab.token` present) copies the lab root key and sets
   `PermitRootLogin prohibit-password`.
6. Update channel: `/etc/pacman.conf` gets `[bifrost]`. Lab: `http://192.168.1.12:8079/bifrost/$arch` (Titan home LAN, served by the `bifrost-repo` unit).
   Release: `https://bifrost.olibuijr.com/arch/$arch`.
7. Copy the media (assets, `/usr/share/bifrost`) from the stage, then `pacman -Sy --noconfirm springboard-shell`
   (the package is installed last so it wins any overlap).
8. Plymouth config (`/etc/plymouth/plymouthd.conf`, `Theme=bifrost`), drop-in
   `springboard.service.d/plymouth.conf` (`After=plymouth-start.service`).
9. Browser: `springboard --install-store-app is.olibuijr.browser.desktop` in the chroot.
10. Account: `useradd -m -G wheel,audio,video,input,storage`, sudo for wheel, root locked,
    `/var/lib/springboard/account` (read by `session` for the local account name).
11. Boot: `bootweave.efi` → `\EFI\BOOT\BOOTx64.efi`, `cmdline.txt` (below), `efibootmgr -c -L BifrOSt`,
    mkinitcpio preset `linux`.
12. Services (chroot): mask `getty@tty1`, `plymouth-quit`, `plymouth-quit-wait`; enable
    `bifrost-update-guard`, `springboard`, `bifrost-splash-handoff`, `NetworkManager`, `sshd`, `seatd`,
    `power-profiles-daemon`.
13. Browser store app, then the summary screen. Progress stages: `Preparing the disk`,
    `Installing the system`, `Setting up your account and apps`, `Finishing`.

Known gap (found 2026-10-08): assets are not in the package. An install whose stage lacks
`/usr/share/springboard/assets` starts a shell that exits with `No such file or directory`. The
fix (copy assets before pacman) is in `plan.rs` but not yet verified on hardware.

## 3. Boot chain (installed PC)

1. UEFI runs `\EFI\BOOT\BOOTx64.efi` (`bootweave.efi`). The NVRAM entry is `BifrOSt`.
2. `cmdline.txt` (`plan::cmdline_txt`):
   `initrd=\initramfs-linux.img root=UUID=<root> rootflags=subvol=@system rw quiet splash loglevel=3 rd.udev.log_level=3 rd.systemd.show_status=false systemd.show_status=false vt.global_cursor_default=0 i915.enable_fbc=0 i915.enable_psr=0 plymouth.use-simpledrm`
   (the i915 flags stop Skylake framebuffer-compression stripes).
3. initramfs (mkinitcpio `linux` preset) mounts `@system` and switches root.
4. Plymouth starts with theme `bifrost` (`bifrost.script`: logo and bar fade in; `Quit` fades them out).
5. `bifrost-installer.service` retains the splash frame until the native installer is ready, then releases DRM and owns the card directly.
6. `springboard.service` (after `plymouth-start`, `seatd`, `systemd-user-sessions`; `Conflicts=getty@tty1`)
   runs `/usr/lib/bifrost/session`:
   - reads the account from `/var/lib/springboard/account`, cleans `/run/bifrost/user/{wayland-*,session.json,shell-ready}`;
   - starts `seatd`; starts `labwc --config-dir /usr/share/bifrost/wayland` as the account;
   - waits up to 10 s for `wayland-0` and `session.json`; then starts `/usr/bin/springboard`.
7. The shell writes `shell-ready` after its first committed Wayland frame (`springboard/src/bare/wayland.rs`,
   `mark_first_frame`).
8. `bifrost-splash-handoff.service` (oneshot, after `springboard.service`) waits up to 60 s for `shell-ready`,
   then `plymouth quit --retain-splash`. It exits 0 when the session already released the splash.

Boot timing: `session` and the handoff append `<uptime-seconds> <mark>` lines to `/var/log/bifrost-boot-timing.log`
(`session-start`, `splash-released`, `compositor-ready`, `shell-launch`, `shell-ready-seen`, `handoff-start`,
`splash-already-gone`). Measured on Midget (0.4.37): firmware 9.4 s, loader 0.6 s, kernel 0.9 s, initrd 3.3 s,
userspace 5.0 s, `graphical.target` at 4.98 s. The tty/serial/tpm device units at the top of `systemd-analyze blame`
(~4 s) are not on the critical chain to `graphical.target` (`NetworkManager` -> `systemd-user-sessions` ->
`springboard.service`), so masking them gains nothing. labwc has no start-up mode option; a `<mode>` element in `rc.xml`
stops it launching.

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

`springboard/src/bare/control.rs`, TCP `7777` (override `SPRINGBOARD_CONTROL_PORT`). Line commands:
`state`, `shot [N]` (1-8), `tap X Y`, `swipe X0 Y0 X1 Y1 [MS]` (alias `drag`), `key ...`, `trace on|off`.
Coordinates are original panel pixels. `tools/midget.py` sends each line over SSH to `127.0.0.1:7777` on Midget (`cmd_control`); the port is not exposed on the lab network.

## 6. Endpoints

| Address | Service | Owner |
|---|---|---|
| `10.99.0.1:8078` | lab sync (installer and shell for lab hosts) | `tools/lab-push.sh` |
| `192.168.1.12:8079/bifrost/x86_64` | lab pacman repo (`springboard-shell`) | `tools/pc-publish.py` |
| `10.99.0.1` `eno2` | iVentoy PXE/netboot (ISO folder) | titan |
| `192.168.20.140:22` | Midget sshd (lab key on lab installs only) | installed system |
| `192.168.20.140:8651` | `bifrost-lab` API (lab ISO only) | live medium |
| `192.168.20.140:7777` | shell control socket | `springboard` |
| `https://bifrost.olibuijr.com/arch/$arch` | release pacman repo | release publish |
| `192.168.1.12` | Titan (lab host, Midget control scripts run here) | titan |

## 7. CLI tools (`tools/`)

| Tool | Use | Notes |
|---|---|---|
| `bifrost pc midget status|state|screen|tap|swipe|key|trace|update|logs|bootnext|reboot|poweroff|wake|bios|me|amt|hw|efi|ssh` | Day-to-day control of Midget | Wraps `tools/midget.py`. Input needs explicit approval per test. |
| `tools/midget.py wipe --disk DEV --confirm "WIPE 10.99.0.200 DEV" [--dry-run]` | Wipe and reinstall Midget | Calls `tools/midget-wipe.py`. |
| `tools/midget-wipe.py` | Guarded erase and reinstall | Preflight: MAC `00:23:24:dc:c8:40`, exactly one erasable disk equal to `--disk`, root must not be that disk, exact token. Steps: PXE to live medium, `BIFROST_AUTO` install, BootNext to disk, verify ssh and `springboard`. Logs in `out/lab/wipes/<stamp>/`. |
| `tools/build-iso.sh [--lab|--debug|--release] [--deploy]` | Build the live medium | Needs sudo. Only for boot-path changes. |
| `tools/lab-push.sh` | Push installer and shell to lab hosts | No ISO rebuild. |
| `tools/pc-publish.py [--release] [--skip-build]` | Build and publish `springboard-shell` | Fast profile. `bifrost pc publish` is the same. Version is `Cargo.toml` plus a timestamp. |
| `tools/pc-app.py`, `pc-app-smoke.py` | Desktop app build and publish | See `docs/bifrost-pc-workflow.md`. |
| `tools/vm-install.sh`, `vm-cycle.py` | VM install loop | `vm-cycle` wipes by simulated keystrokes (fragile). |
| `tools/iventoy-prune.sh` | Prune old ISOs in iVentoy | Keeps the newest 2 and anything under 4 h old. Never delete an ISO a machine booted from. |
| `tools/test_midget.py`, `tools/test_midget_wipe.py` | Unit tests for the scripts above | Run without touching hardware. |

## 8. Procedures

**Known failure chain (2026-10-08):** a shell start that fails for any reason (missing assets, labwc swapchain
rejected on DP-2, a stale DRM holder) counts toward the crash guard. Three of them latch the desktop off until the counter
is cleared. Diagnose with `journalctl -b -u springboard`, then run the shell as root against a live labwc, since the
session starts it as root and its data lives in `/var/lib/springboard`. Running it as the account reads the wrong data dir.
Intel Skylake stripes on the splash came from i915 framebuffer compression. The installed kernel command line now sets
`i915.enable_fbc=0 i915.enable_psr=0 plymouth.use-simpledrm`.

**Ship a shell or desktop change (no ISO):** bump `springboard/Cargo.toml` and `CHANGELOG.md`,
`tools/pc-publish.py`, then Midget picks it up through Software Update.

**Ship an installer change:** `tools/lab-push.sh`. The next netboot of a lab host runs it.

**Change boot-time files (splash, handoff, theme, cmdline, initramfs):** the package carries the theme and
handoff unit, but the boot loader and cmdline come from the live medium. That needs `tools/build-iso.sh --lab`
and a new ISO in iVentoy. Confirm with the user first.

**Wipe Midget:** `tools/midget.py wipe --disk /dev/nvme0n1 --confirm "WIPE 10.99.0.200 /dev/nvme0n1" --dry-run`,
then the same command without `--dry-run`. Run the wipe as a foreground job, or background it with
`setsid`. Plain `nohup` dies when the calling shell times out, which happened on 2026-10-08.
Review `out/lab/wipes/<stamp>/log.txt` and `report.json`.

**Diagnose a Midget that does not show the desktop:** `pacman -Q springboard-shell`, `systemctl status springboard`,
`journalctl -b -u springboard`, then run `/usr/bin/springboard` by hand as the account with
`XDG_RUNTIME_DIR=/run/bifrost/user WAYLAND_DISPLAY=wayland-0` (labwc must be up).
