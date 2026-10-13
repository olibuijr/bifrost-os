# BifrOSt boot-to-desktop design

This branch moves the PC toward one BifrOSt-owned experience from firmware
handoff to the desktop. The goal is macOS-like continuity: one visual identity,
one recovery model, no text-console flash, and no generic boot menu exposed in
the normal path.

## Boundary

UEFI is still the motherboard firmware. BifrOSt begins at the UEFI application
handoff. The Rust EFI application owns the first screen, boot entry selection,
rollback choice, kernel launch, and removable-media discovery. Linux remains
the kernel and hardware/runtime layer; the window server is BifrOSt's own (`bifrost-wm`). Calling the current system
“pure Rust from BIOS” would still be misleading, because the kernel and drivers are Linux.

## Current change

`springboard/bootweave/efi` is now the source of truth for both ISO and installed
recovery boot binaries. `tools/build-efi-boot.sh` produces the same artifact at
both destinations. The loader prefers `EFI/BIFROST/boot.cfg` on installed systems and accepts
`EFI/BOOT/cmdline.txt` on removable media. The legacy `\bifrost\cmdline.txt` and `EFI\BIFROST\cmdline.txt` are gone
(`docs/boot-architecture.md`). Installed systems get a stable BifrOSt namespace.

## Status (2026-10-13)

- Boot loader: the Rust EFI loader is the only boot program. Installed disks register `\EFI\BIFROST\BOOTx64.efi`
  and boot `EFI/BIFROST/boot.cfg`; `EFI/BOOT` is the fallback copy. The ISO is UEFI-only and its El Torito ESP holds
  just the loader, kernel and initramfs (BLS entries remain as data for iVentoy, not a boot loader).
- Rollback: the loader boots `EFI/BIFROST/rollback.txt` once `pending` is set and unaccepted
  (`springboard/bootweave/efi/src/boot_policy.rs`, `packaging/arch/recovery/system.rs`).
- Splash: Plymouth is gone; Bootweave supervises the session behind the EFI/simpledrm frame.
- Done: the installed compositor is `bifrost-wm`; labwc is removed (2026-10-13), with safe mode and the recovery flow
  as the fallback (`docs/window-server.md`).
- The boot frame is now one screen without a progress bar; the loader can unlock an encrypted root; the loader boots a
  unified kernel image. See `docs/boot-architecture.md` and `docs/boot/`.

## Migration order (history; steps 1 to 4 are done except generic session glue)

1. Rust EFI boot application: display handoff, USB/removable discovery,
   signed/current/previous generation selection, and recovery UI.
2. Rust early userspace: replace Plymouth and shell scripts with a small
   BifrOSt init/handoff service while retaining Linux drivers and initramfs
   facilities that are needed for hardware bring-up.
3. BifrOSt compositor: replace labwc as the session compositor. Done: `bifrost-wm`.
4. Remove Plymouth, labwc, and generic session glue only after the equivalent
   Rust paths have hardware-tested USB, installed-disk, rollback, hotplug, and
   desktop startup flows.

Plymouth and labwc are removed. The session script and systemd units remain until a Rust supervisor owns the
same stages (`docs/native-bifrost-stack.md`).

## Boot benchmarks

`tools/boot-bench.py live|installed [--runs N] [--inject] [--esp IMG] [--record]` boots the OVMF VM and reports
medians per step; `--record` appends them to `docs/perf-history.jsonl` (host `titan-vm`). No ISO rebuild is needed:
`--inject` pushes the working tree (live: `tools/lab-push.sh` feeds lab-sync; installed: scp into the disk).

| Step | Source |
|---|---|
| firmware, loader | the BifrOSt loader's `LoaderTimeInitUSec`/`LoaderTimeExecUSec` (Boot Loader Interface), via `systemd-analyze` |
| kernel, initrd, userspace | `systemd-analyze` |
| milestones | `/run/bifrost/boot-marks`, ms since kernel start (`springboard::bootsplash::mark`) |
| modesets after the loader | KMS lines that *selected* a mode (not inherited) plus compositor modeset lines in the journal |

## One modeset, one fade

The loader sets the panel's largest GOP mode, the boot's only modeset. simpledrm and the GPU driver (fastboot)
inherit it; the installer's KMS code keeps the CRTC's active mode (`kms.rs`, "inherited" in the journal) unless
`SPRINGBOARD_MODE` asks for another. The first KMS frame is the loader's frame redrawn (`bootsplash::paint`), and the
first screen cross-fades from it over 450 ms: the installer in its loop; on the desktop `bifrost-wm` does the fade
(`wm/src/boot.rs`, `BIFROST_BOOT_FADE`, first session of a boot only). `kms::Display::present` still fades for the
bare (no compositor) path.

## Boot path decisions (2026-10-10)

- Installed initramfs: `HOOKS=(base udev autodetect microcode modconf kms keyboard block filesystems bifrost-mounts)`.
  The systemd initramfs and PID 1 wait out systemd's mount-event rate limit (~0.9 s); busybox init plus the
  `bifrost-mounts` late hook mounts every fstab subvolume and the ESP before switch_root.
- springboard.service and seatd (`seatd-early.conf`) have no default dependencies; seatd's PrivateTmp is off because
  it pulled in local-fs.target. The live installer keeps `After=sysinit.target`: taking tty1 earlier hung console units.
- Live initramfs: no microcode or memdisk, and `bifrost-live-trim` drops server NIC/HBA/filesystem modules.
