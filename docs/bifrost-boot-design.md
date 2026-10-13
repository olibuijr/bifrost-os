# BifrOSt boot-to-desktop design

This branch moves the PC toward one BifrOSt-owned experience from firmware
handoff to the desktop. The goal is macOS-like continuity: one visual identity,
one recovery model, no text-console flash, and no generic boot menu exposed in
the normal path.

## Boundary

UEFI is still the motherboard firmware. BifrOSt begins at the UEFI application
handoff. The Rust EFI application owns the first screen, boot entry selection,
rollback choice, kernel launch, and removable-media discovery. Linux remains
the kernel and hardware/runtime layer while the native BifrOSt compositor is
being developed; calling the current system “pure Rust from BIOS” would be
misleading until that replacement exists.

## Current change

`springboard/bootweave/efi` is now the source of truth for both ISO and installed
recovery boot binaries. `tools/build-efi-boot.sh` produces the same artifact at
both destinations. The loader prefers `EFI/BIFROST/boot.cfg`, then accepts
and `EFI/BOOT/cmdline.txt` on removable media (the legacy `EFI/BIFROST/cmdline.txt` and
path. This keeps existing USB media bootable while giving installed systems a
stable BifrOSt namespace.

## Status (2026-10-10)

- Boot loader: the Rust EFI loader is the only boot program. Installed disks register `\EFI\BIFROST\BOOTx64.efi`
  and boot `EFI/BIFROST/boot.cfg`; `EFI/BOOT` is the fallback copy. The ISO is UEFI-only and its El Torito ESP holds
  just the loader, kernel and initramfs (BLS entries remain as data for iVentoy, not a boot loader).
- Rollback: the loader boots `EFI/BIFROST/rollback.txt` once `pending` is set and unaccepted
  (`springboard/bootweave/efi/src/boot_policy.rs`, `packaging/arch/recovery/system.rs`).
- Splash: Plymouth is gone; Bootweave supervises the session behind the EFI/simpledrm frame.
- Done: the installed compositor is `bifrost-wm`; labwc is removed.

## Migration order

1. Rust EFI boot application: display handoff, USB/removable discovery,
   signed/current/previous generation selection, and recovery UI.
2. Rust early userspace: replace Plymouth and shell scripts with a small
   BifrOSt init/handoff service while retaining Linux drivers and initramfs
   facilities that are needed for hardware bring-up.
3. BifrOSt compositor: replace labwc as the session compositor. The shell must
   own the display server before labwc can be removed because packaged apps
   currently connect through Wayland.
4. Remove Plymouth, labwc, and generic session glue only after the equivalent
   Rust paths have hardware-tested USB, installed-disk, rollback, hotplug, and
   desktop startup flows.

The removal is intentionally staged: deleting those packages now would break
the existing desktop rather than extend BifrOSt.

## Boot benchmarks

`tools/boot-bench.py live|installed [--runs N] [--inject] [--esp IMG] [--record]` boots the OVMF VM and reports
medians per step; `--record` appends them to `docs/perf-history.jsonl` (host `titan-vm`). No ISO rebuild is needed:
`--inject` pushes the working tree (live: `tools/lab-push.sh` feeds lab-sync; installed: scp into the disk).

| Step | Source |
|---|---|
| firmware, loader | the BifrOSt loader's `LoaderTimeInitUSec`/`LoaderTimeExecUSec` (Boot Loader Interface), via `systemd-analyze` |
| kernel, initrd, userspace | `systemd-analyze` |
| milestones | `/run/bifrost/boot-marks`, ms since kernel start (`springboard::bootsplash::mark`) |
| modesets after the loader | KMS lines that *selected* a mode (not inherited) plus wlroots `Modesetting` lines |

## One modeset, one fade

The loader sets the panel's largest GOP mode, the boot's only modeset. simpledrm and the GPU driver (fastboot)
inherit it; the installer's KMS code keeps the CRTC's active mode (`kms.rs`, "inherited" in the journal) unless
`SPRINGBOARD_MODE` asks for another. The first KMS frame is the loader's frame redrawn (`bootsplash::paint`), and the
first screen cross-fades from it over 450 ms: the installer in its loop, the desktop in `kms::Display::present` when
the session sets `BIFROST_BOOT_FADE` (first session of a boot only).

## Boot path decisions (2026-10-10)

- Installed initramfs: `HOOKS=(base udev autodetect microcode modconf kms keyboard block filesystems bifrost-mounts)`.
  The systemd initramfs and PID 1 wait out systemd's mount-event rate limit (~0.9 s); busybox init plus the
  `bifrost-mounts` late hook mounts every fstab subvolume and the ESP before switch_root.
- springboard.service and seatd (`seatd-early.conf`) have no default dependencies; seatd's PrivateTmp is off because
  it pulled in local-fs.target. The live installer keeps `After=sysinit.target`: taking tty1 earlier hung console units.
- Live initramfs: no microcode or memdisk, and `bifrost-live-trim` drops server NIC/HBA/filesystem modules.
