# Task 04: unified kernel image and a per-machine initramfs

Contract: `docs/boot-architecture.md`, "Prepared, not probed". Task: `tasks/boot/04-uki-and-initramfs.md` (main checkout,
git-ignored). Branch `boot/04-uki-and-initramfs`. Nothing here was built or run by the agent (owner rule: the lead builds
once after the waves merge); every item below is checked by reading only.

## What is implemented

1. **Unified kernel image `EFI/BIFROST/bifrost.efi`** (`packaging/arch/recovery/system.rs`, `build_uki`).
   Built with `ukify build` from `/boot/vmlinuz-linux`, every `*-ucode.img` (sorted), `/boot/initramfs-linux.img`, and
   the boot line's options (the `initrd=` and `uki=` tokens are dropped; everything else keeps its order). The output is
   written to `bifrost.efi.new` and renamed in place only after ukify succeeded. The old image becomes
   `bifrost-previous.efi`. `boot.cfg` is rewritten last (`uki=\EFI\BIFROST\bifrost.efi <options>`), so a failed build
   keeps the last good boot line. An unchanged image is detected by byte comparison and nothing is touched.
2. **Build triggers**: pacman hook `packaging/arch/recovery/95-bifrost-uki.hook` (PostTransaction on kernel install or
   upgrade, runs `bifrost-update-guard uki`); the installer (`springboard/src/installer/plan.rs`, after
   `mkinitcpio -p linux`, failure ignored so the kernel-plus-initramfs boot line remains); `uki` is a new arm of
   `packaging/arch/recovery/guard.rs`. The package installs the hook (`package-files.sh`) and `systemd-ukify` is added to
   `runtime-packages.txt`.
3. **Loader start path** (`springboard/bootweave/efi`): `src/uki.rs` (new, pure) finds the `uki=` token. `main.rs` starts
   such an image directly with `start_file` (the PE is loaded from its device path and started with the boot line minus
   the token as its load options). Lines without the token keep the old kernel-plus-initrd path (`boot_line` calls
   `start_file` for both). A `uki=` image that fails to start falls back to `rollback.txt` even without a pending marker.
4. **Rollback**: `prepare()` copies the running `bifrost.efi` to `bifrost-previous.efi` before an update; `fallback_line`
   writes `rollback.txt` as `uki=\EFI\BIFROST\bifrost-previous.efi`, the `@rollback` subvolume and `bifrost.recovery=1`
   (older installs still get the previous kernel and initramfs line). `restore()` copies `bifrost-previous.efi` back to
   `bifrost.efi` before the boot line is restored.
5. **Per-machine modules**: `bifrost-modules-record.service` (after `springboard.service`) waits up to 10 minutes for
   `desktop-ready` or `desktop-first-ui` in `/run/bifrost/boot-marks`, then runs `bifrost-update-guard record-modules`.
   That records the `/proc/modules` names (`STORE/modules`) and a hardware fingerprint (FNV-1a over PCI `vendor:device`
   and the DMI board name, `STORE/hardware`). When the list or the fingerprint changed it rewrites
   `/etc/mkinitcpio.conf.d/bifrost-modules.conf` (`MODULES=(...)`), rebuilds the initramfs with `mkinitcpio -p linux`,
   and rebuilds the UKI. On a hardware change the recorded list is ignored and the drop-in is removed (generic set).
   The installer enables the unit (`springboard.install`). The unit runs `Nice=19` and idle I/O.

## Mapping to the contract

| Contract line | Where |
|---|---|
| `EFI/BIFROST/bifrost.efi`: kernel, initramfs, command line, microcode | `build_uki` |
| Loader starts it directly, old kernel plus initrd as fallback | `efi/src/main.rs` `boot_line`, `uki.rs` |
| Minimal initramfs from the modules used on the previous boot | `record_modules`, `reconcile_modules` |
| Generic set after a hardware change | `hardware_fingerprint`, `recorded_modules` |
| Rollback kept | `bifrost-previous.efi`, `prepare`, `restore`, `fallback_line` |
| Caches for the first frame (task 08) | not part of this task |

## Tests (hand-checked, not run)

- `system.rs` `uki_tests`: 6 tests (uki line and idempotence, fallback line, module names sorted/unique/plain,
  modules drop-in text, fingerprint order-free and hardware-sensitive, empty fingerprint = FNV offset `cbf29ce484222325`).
- `efi/src/uki.rs` tests: 5 tests (token position, kernel lines and empty values, option stripping and order, spacing,
  rollback line). The efi crate is a `no_std` UEFI binary outside the host workspace, so these tests only compile when
  someone runs them with a host target harness; they are kept for the same reason the other efi module tests exist.

## Open items and risks (for the lead and Midget)

- `mkinitcpio` must read `/etc/mkinitcpio.conf.d/*.conf`. If the installed mkinitcpio does not, the drop-in is ignored and
  the initramfs keeps the generic set (the boot still works). Check on Midget before relying on it.
- `ukify build` flags (`--linux`, `--initrd` repeated, `--cmdline`, `--output`) assume a systemd version with the `build`
  subcommand; the stub is auto-detected from `/usr/lib/systemd/boot/efi/linuxx64.efi.stub`.
- The loader passes the boot-line options as load options; systemd-stub uses them instead of the cmdline embedded by
  ukify. The embedded cmdline is the same string, so this is intentional but should be confirmed in the Midget log.
- The `fallback_line` for an image rollback keeps `rootflags=subvol=@rollback`, as before.
- `bootweave.efi` in `packaging/arch/recovery/` and `packaging/archiso/` is rebuilt by `tools/build-efi-boot.sh`; this task
  needs no script change.
- Not done: task 01's baseline (kernel start to Bootweave start) does not exist yet, so the "drops below the baseline"
  check is left to the lead.

## Midget verification (lead)

1. Build and publish as usual (`bifrost pc publish`), then `bifrost pc midget ota` until `phase=accepted`.
2. On Midget as root: `ls -l /boot/EFI/BIFROST/` shows `bifrost.efi`; `cat /boot/EFI/BIFROST/boot.cfg` starts with
   `uki=\EFI\BIFROST\bifrost.efi`; `bifrost-update-guard uki` is a no-op (second run prints nothing and does not change
   `bifrost.efi`, check with `sha256sum`).
3. Reboot. The loader log (`systemd-analyze` shows the stub, `LoaderTimeExecUSec` is set) shows the UKI as the started image,
   and `cat /proc/cmdline` has no `initrd=` token. Record kernel start to Bootweave start from the boot marks.
4. After desktop-ready: `cat /var/lib/bifrost-recovery/modules | wc -l`, `cat /var/lib/bifrost-recovery/hardware`, and
   `cat /etc/mkinitcpio.conf.d/bifrost-modules.conf` (a `MODULES=(...)` line). Confirm `lsinitcpio /boot/initramfs-linux.img`
   lists those modules.
5. Rollback: run `bifrost-update-guard prepare`, change `boot.cfg` to a bad `uki=` path, reboot; the loader must fall back
   to `rollback.txt` (`uki=\EFI\BIFROST\bifrost-previous.efi`). Then `bifrost-update-guard rollback` and confirm `bifrost.efi`
   equals the copy in `bifrost-previous.efi`.
6. Hardware-change path (only if available): change `/var/lib/bifrost-recovery/hardware` to a wrong value, run
   `bifrost-update-guard record-modules`; the drop-in must be removed and the initramfs rebuilt.
