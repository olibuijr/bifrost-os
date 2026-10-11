# Arch VM test harness (`tools/arch-vm.sh`)

An Arch Linux x86_64 guest under QEMU/KVM that runs the SpringBoard shell as the sole UI on tty1 with the
desktop profile, through the real KMS path on a virtio-gpu (no `SPRINGBOARD_VIRTUAL`). Packaging for the
same unit lives in `packaging/arch/`.

## Run

```
tools/arch-vm.sh install-deps        # sudo pacman: qemu-desktop edk2-ovmf arch-install-scripts (skips what is present)
tools/arch-vm.sh create              # pacstrap base+linux+openssh into a 6G raw image (sudo for loop mount/pacstrap, ~1-2 min)
tools/arch-vm.sh start [--headed]    # KVM, headless by default (-display none); waits for ssh (about 25 s)
tools/arch-vm.sh deploy              # copies springboard/target/release/shell, the unit, assets; restarts the unit
tools/arch-vm.sh shot home.png       # QMP screendump (PNG) into ~/Pictures/tab/ (absolute paths are kept)
tools/arch-vm.sh key esc | tap X Y | swipe X0 Y0 X1 Y1 [MS]
tools/arch-vm.sh ssh 'journalctl -u springboard -n 30 --no-pager'
tools/arch-vm.sh status | stop
```

`deploy` takes `SPRINGBOARD_BIN` (default `<repo>/springboard/target/release/shell`) and `SPRINGBOARD_ASSETS`
(default `<repo>/springboard/assets`; the dir is untracked, so in a worktree point it at the main checkout's).
Assets go to `/usr/share/springboard/assets` once (marker `.deployed`) and seed `/var/lib/springboard` on the
first service start (marker `.seeded`); delete `/var/lib/springboard/.seeded` in the guest to reseed.

## Layout, ports, knobs

- Everything under `~/.local/share/arch-vm/` (`ARCH_VM_DIR`): `arch.img` (raw root, `root=/dev/vda`), `vmlinuz` +
  `initramfs.img` (direct kernel boot, no bootloader or UEFI needed), `id_ed25519` (root key), `qmp.sock`,
  `qemu.log`, `serial.log` (guest console, ttyS0), `tmp/` (TMPDIR; nothing goes to /tmp).
- Network: user-mode only; guest ssh is `127.0.0.1:2250` (`ARCH_VM_SSH_PORT`), root with the key above.
- Display: `virtio-vga` 1200x1920 (`ARCH_VM_W`/`ARCH_VM_H`), 4 CPUs, 1 GiB (`ARCH_VM_CPUS`, `ARCH_VM_MEM`).
- A virtio balloon with free-page reporting returns unused guest pages to the host without reducing guest-visible RAM. No balloon target is imposed. Use `ARCH_VM_MEM=3072` for heavier workloads; the 1 GiB default is for shell development.
  `tap`/`swipe` take guest pixels for those dimensions and go through a virtio absolute pointer (absolute 0..32767).
- Input devices in the guest are named `QEMU Virtio Keyboard` and `QEMU Virtio Tablet`. No audio devices.
- The unit is `packaging/arch/springboard.service` (root, tty1, `Conflicts=getty@tty1.service`,
  `SPRINGBOARD_PLATFORM=desktop`, `Restart=on-failure`).

## Gotchas

- The pacman mkinitcpio hook fails inside pacstrap's chroot, so `create` builds `initramfs.img` itself with
  `-S autodetect` (all modules, so virtio_gpu/virtio_blk are present) and copies the kernel out of
  `/usr/lib/modules`. After a guest kernel update re-run `create` on a fresh image (delete `arch.img`).
- `pacstrap -K` leaves gpg-agent holding the mount, so `create` runs `fuser -k -m` before `umount`.
- `key` names are QEMU qcodes (`esc`, `ret`, `home`, `volume_up`, ...); `power` maps to qcode `power`.
- `shot` of a black screen means the shell is not drawing (check `status`/journal), not a harness failure.

## Installer ISO

The installer ISO and its native installer are described in `docs/installer.md`. This document covers the
direct-kernel dev VM only (`tools/arch-vm.sh`); the earlier yad/Openbox installer and the Limine boot plan it
described no longer exist.
