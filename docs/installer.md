# BifrOSt installer (x86_64 PC)

A native Rust installer on the live ISO. It replaces the earlier bash+YAD, web-UI and Limine/OpenCore generations.

## Parts

| Piece | Where |
| --- | --- |
| Screens, input, partition choices | `springboard/src/installer/app.rs`, `disks.rs`, `keymaps.rs`, `video.rs` |
| The install itself (partition, pacstrap, configure) | `springboard/src/installer/plan.rs` |
| Main loop on KMS + evdev | `springboard/src/bin/installer.rs` (`bifrost-installer`) |
| Keyboard layouts | `tools/gen_keymaps.py` → `keymaps_data.rs` (xkeyboard-config via libxkbcommon) |
| BifrOSt boot application (UEFI) | `springboard/bootweave/efi` (Rust, `x86_64-unknown-uefi`) → `tools/build-efi-boot.sh` → ISO and recovery payloads |
| Unattended lab reinstall | `springboard/src/installer/unattended.rs` |
| First-frame caches for the installed panel | `springboard/src/first_frame_cache.rs` (written into the target before unmount) |
| Logo assets | `tools/bifrost_brand.py` |
| ISO profile overlay | `packaging/archiso/airootfs`, applied by `tools/apply-archiso-profile.py` |
| ISO build | `tools/build-iso.sh`, `tools/prepare-mkarchiso.py` (adds the `uefi.bifrost` boot mode) |

## Boot path

UEFI runs `EFI/BOOT/BOOTx64.efi` (the BifrOSt Rust boot application, Bootweave). It draws the boot frame (the mark alone),
reads `\EFI\BIFROST\boot.cfg` first and then the removable-media compatibility paths, and starts the kernel from the
directory of the `initrd=` file (the kernel name follows the initramfs name); the EFI stub loads the initramfs, so the
frame stays up while it is read. There is no Plymouth: `bifrost-installer.service` launches the native Rust
installer directly on DRM/KMS and redraws the same frame first (the installed desktop uses the `bifrost-wm` session
separately). On an installed disk the same loader boots the unified kernel image or the kernel from the ESP (`/boot`),
command line from the installer-written `boot.cfg`.

## Installing

Modes: **Erase disk** (GPT, 512 MiB FAT32 ESP + Btrfs root) and **Install next to what is there** (largest free region of a
GPT disk; an existing FAT ESP of at least 400 MiB is reused, otherwise a new one is made). Erasing asks for
confirmation first. The installer copies the files it needs from the live medium into RAM before it touches the
disk, creates a locked-root system with one wheel user (password via `chpasswd` on stdin), writes locale,
console keymap and X11 keyboard from the chosen layout, enables SpringBoard, NetworkManager and sshd.

Release ISOs install without network access. When `/usr/share/bifrost/offline/manifest.json` is present, the installer
verifies the bundle before partitioning, uses only its file-backed pacman repository for pacstrap, then installs the
bundled `springboard-shell` and the Browser and Files app packages. An advertised but incomplete or corrupt bundle is a
hard error; it never falls back to the network. The manifest uses schema version 1: `repo` identifies `repo/offline.db`
and its SHA-256, `packages` lists repository archive basenames and SHA-256 checksums, `shell` identifies the bundled
shell archive and checksum, and `apps` identifies the two app package paths (`apps/<id>-<version>.aap`) and checksums.
Pacman's temporary file-repository configuration is not copied into the installation; the installed system keeps the
normal Arch sources plus the `[bifrost]` update repo. The ISO bundle also includes dependency archives, which the
installer checks against pacman's resolved package plan, using an empty temporary package database, before any disk changes.

A lab install with the `mirror` config field set retains its existing route: pacstrap uses only the lab mirror
(`offline_pacman_conf`, `http://10.99.0.1:8078/mirror/`, built by `tools/lab-mirror.sh`), and the installed system
keeps the stock configuration plus the `[bifrost]` update repo.

The installed kernel is `linux-rt` (stock `linux` for NVIDIA or when the package source lacks `linux-rt`). After mkinitcpio the
installer runs `bifrost-update-guard uki` to build the unified kernel image `EFI/BIFROST/bifrost.efi`; a failure keeps the
kernel-plus-initramfs boot line. The installer also writes the chosen look into `settings.ini` and builds the first-frame
caches for the panel.

## Developing

- Logic is unit-tested: `cd springboard && cargo test --lib installer`.
- On a dev VM: `BIFROST_NO_VIDEO=1 BIFROST_DISK_FIXTURE=file SPRINGBOARD_MODE=1920x1080@60 bifrost-installer`

Before building an ISO, prepare the generated `springboard/assets` directory (including its required fonts, default icons,
wallpaper, and wallpaper index); `tools/build-iso.sh` copies it into the live image and fails closed if those files are
missing. The ISO builder does not fetch or generate these assets. Installer footage is staged from
`packaging/archiso/airootfs/usr/share/bifrost/video`; lab builds may explicitly select a prepared directory with
`BIFROST_ISO_VIDEO_DIR`.
  (`BIFROST_DISK_FIXTURE` holds `parted -sm` outputs separated by `---`; `BIFROST_LAYOUT=is` picks a layout).
- Lab hooks (only when `/run/bifrost-shot-on` exists): the installer writes `/run/bifrost-screen.png` and reads input lines
  (`down|move|up X Y`, `key CODE 1|0`) from `/run/bifrost-input`.

## ISO builds

`tools/build-iso.sh --lab` adds a root SSH key (`out/lab/id_ed25519`) and the `bifrost-lab` API (screenshot, key, type, click,
exec on port 8651, header `X-Lab-Token`, token in `out/lab/token`). `--release` strips all of that.
`tools/check-iso-hardening.sh ISO` fails if an ISO carries lab artifacts. `--deploy` copies the ISO into iVentoy under a new
name; never overwrite an image a machine is booted from (it causes I/O errors mid-install).

## Netboot lab

iVentoy (`out/iventoy`, bound to titan's `eno2` only) serves ISOs by PXE; target machines boot them as a virtual CD.
The lab cable network is 10.99.0.0/24; `tools/lab-up.sh` (run after a titan reboot) sets the lab NIC address, enables
IPv4 forwarding and adds an nft table `bifrost_nat` that masquerades 10.99.0.0/24 out through `enp6s0` (`lab-up.sh down` removes it).
Titan also serves the lab pacman repo and its package mirror on :8079 (see below).

## Package mirror (:8079)

`bifrost pc publish` (`tools/pc-publish.py`) publishes `springboard-shell` into `out/pc-repo/bifrost/x86_64` and then runs
`tools/pc_mirror.py`. The mirror (`out/pc-repo/mirror/$repo/os/$arch`, same :8079 server) holds the core and extra databases and every
package file BifrOSt systems install, resolved from the package set of `tools/lab-mirror.sh --list`, so a guarded `pacman -Su`
on Midget never meets a 404. It is incremental: only missing files are fetched, and a database is replaced only after every
file it lists is present. `pc_mirror.py --sync|--check` run it by hand; `--skip-mirror-sync` on publish only checks.
The upstream is `BIFROST_UPSTREAM_MIRROR` (default `https://geo.mirror.pkgbuild.com`).

The publisher also exports the retained shell packages and pacman database to
`~/Projects/bifrost-site/site/arch/x86_64`, which the live site serves at
`https://bifrost.olibuijr.com/arch/$arch` for installed-system updates and new non-lab installs. Set
`BIFROST_PUBLIC_SITE` to the static site root on another publishing host. Package files are immutable by version; both
database filenames are replaced atomically after the package files, with `bifrost.db` last.

## Faster iteration

- **Installer changes without an ISO rebuild (lab/debug images):** `tools/lab-push.sh` builds the installer and publishes it,
  with its assets, on titan (`http://10.99.0.1:8078`). A lab image's `bifrost-lab-sync` fetches the newest binary at boot, right
  before `bifrost-installer.service` starts (silently skipped if titan is unreachable). Reboot the target; no ISO needed.
  For VM verification, the same binary can be copied from the working tree into a running lab ISO VM and the service restarted;
  this validates installer changes without rebuilding or booting a new ISO.
  Rebuild the ISO only when packages, the initramfs, units or the boot setup change.
- **Debug images (`--debug`):** graphical splash with debug diagnostics retained in the journal, the journal streamed to titan
  (`tools/lab-log-sink.py` → `out/lab/logs/`), installer trace via `BIFROST_TRACE`.

## Headless install (VM and CI)

`BIFROST_AUTO=user:password:/dev/vda bifrost-installer` runs the whole install without a display and exits 0 on success,
1 with the failure message otherwise (Erase mode, first keyboard layout, en_US.UTF-8). `tools/vm-install.sh install` wraps it
for the QEMU VM. Package set: `packages()` in `installer/plan.rs`; release ISOs carry this set, GPU driver alternatives,
kernel alternatives, package dependency closure, the shell archive and the two default app packages.
See `docs/bifrost-pc-workflow.md`.

Fresh Btrfs setup loads the kernel module before partitioning, mounts the six
subvolumes explicitly, and includes Btrfs in the installed initramfs. Asset
staging merges directory contents into their destinations. Package registration
uses the recovery subvolume because arch-chroot replaces both /run and /tmp.

## Unattended install (lab reinstall)

`bifrost-installer` looks for `EFI/BIFROST/unattended.toml` on the ESP of each candidate disk before it draws anything
(`installer/unattended.rs`). The running system's MCP (`lifecycle reinstall`) writes it, sets BootNext to PXE and reboots;
the netbooted lab ISO then installs with no input and reboots into the new system (it sets BootNext to the new `BifrOSt`
entry). `BIFROST_NO_UNATTENDED=1` skips the search; a refused preset falls through to the on-screen installer.

Why an ESP stash and not a file on titan: the preset carries facts only the running machine knows (stable disk id,
timezone, hostname), the guard is strongest when the file sits on the very disk it authorises erasing, it needs no write
path into titan, and it works with any netboot server. The one secret, the password, stays on titan: `password_url` points
at `http://10.99.0.1:8078/lab/tester.pw`, written by `tools/midget.py reinstall` from `out/lab/tester.password` (untracked,
0600, like `out/lab/id_ed25519`). Anyone on the lab VLAN can read it; it protects a lab account on a machine that already
carries the lab root key.

```toml
version = 1                      # required
disk_id = "eui.e8238fa6..."      # required: lsblk -dno WWN of the disk to erase
mac = "00:23:24:dc:c8:40"        # required: NIC that must be present
hostname = "midget"              # required
password_url = "http://10.99.0.1:8078/lab/tester.pw"   # required, lab hosts only
username = "tester"              # required
full_name = "Tester"
locale = "en_US.UTF-8"           # default en_US.UTF-8
keyboard = "us"                  # keymaps layout id, default us
timezone = "Atlantic/Reykjavik"  # empty keeps UTC
autologin = true                 # default true: no passcode; the lab flow unlocks the lock screen with Enter
mcp = true                       # default true: bifrost-mcp.service is always enabled by the installer
created_epoch = 1760000000       # preset is refused after created_epoch + expires_s
expires_s = 1800
job = "1760000000-reinstall"
```

Unknown keys, a bad username/hostname/timezone or a password host outside the lab are errors. Auto mode refuses when
the disk carrying the preset does not report `disk_id`, the NIC is absent or the preset is stale; refusals happen before
the disk is touched. The lab key and `lab.token` behaviour is unchanged (the tester account gets the lab key too).
Tests: `cargo test --lib installer::unattended` and `cargo test -p bifrost-mcp lifecycle`.
