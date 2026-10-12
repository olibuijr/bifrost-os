#!/usr/bin/env python3
"""Create a repo-local mkarchiso with the BifrOSt boot mode added.

The generated copy lives under out/ and leaves the host's Archiso installation
untouched. The patch is deliberately anchored to mkarchiso's current boot-mode
and xorriso hooks; it fails closed if upstream changes those extension points.
"""
from pathlib import Path
import os
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path("/usr/bin/mkarchiso")
DEST = ROOT / "out/archiso-tools/mkarchiso"
INJECT = r'''
# BifrOSt UEFI mode: the Rust EFI loader is the only boot program on the medium. No systemd-boot, GRUB or
# syslinux: the El Torito ESP holds EFI/BOOT/BOOTx64.efi (firmware fallback path), EFI/BIFROST (the
# installed-system namespace), the kernel + initramfs, and Boot Loader Spec entries that only network boot
# servers (iVentoy) read for the kernel arguments.
_make_bootmode_uefi.bifrost() {
    _msg_info "Setting up the BifrOSt EFI loader..."
    local repo="${BIFROST_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
    # tools/build-efi-boot.sh writes packaging/archiso/bootweave.efi (untracked); the recovery copy is tracked
    local splash="${repo}/packaging/archiso/bootweave.efi"
    [[ -s "${splash}" ]] || splash="${repo}/packaging/arch/recovery/bootweave.efi"
    local extra="quiet loglevel=3 rd.udev.log_level=3 udev.log_level=3 rd.systemd.show_status=false systemd.show_status=false systemd.log_target=journal rd.luks=0 rd.lvm=0 rd.md=0 rd.dm=0 vt.global_cursor_default=0 systemd.tty.rows.console= i915.enable_fbc=0 i915.enable_psr=0 threadirqs"
    # debug builds put their own command line (journal diagnostics with graphical splash) in <profile>/bifrost-cmdline
    [[ -s "${profile}/bifrost-cmdline" ]] && extra="$(cat "${profile}/bifrost-cmdline")"
    local line="initrd=\\${install_dir}\\boot\\${arch}\\initramfs-linux.img archisobasedir=${install_dir} archisosearchuuid=${iso_uuid} ${extra}"
    rm -rf -- "${isofs_dir}/EFI" "${isofs_dir}/loader" "${isofs_dir}/shell"*.efi
    install -d -m 0755 -- "${isofs_dir}/EFI/BOOT" "${isofs_dir}/EFI/BIFROST" "${isofs_dir}/loader/entries"
    install -m 0644 -- "${splash}" "${isofs_dir}/EFI/BOOT/BOOTx64.efi"
    install -m 0644 -- "${splash}" "${isofs_dir}/EFI/BIFROST/BOOTx64.efi"
    printf '%s\n' "${line}" > "${isofs_dir}/EFI/BOOT/cmdline.txt"
    printf '%s\n' "${line}" > "${isofs_dir}/EFI/BIFROST/boot.cfg"
    local opts="archisobasedir=${install_dir} archisosearchuuid=${iso_uuid} ${extra}"
    printf 'title   Install BifrOSt\nsort-key 01\nlinux   /%s/boot/%s/vmlinuz-linux\ninitrd  /%s/boot/%s/initramfs-linux.img\noptions %s\n' "${install_dir}" "${arch}" "${install_dir}" "${arch}" "${opts}" > "${isofs_dir}/loader/entries/01-bifrost.conf"
    printf 'title   Try BifrOSt\nsort-key 02\nlinux   /%s/boot/%s/vmlinuz-linux\ninitrd  /%s/boot/%s/initramfs-linux.img\noptions %s bifrost.live=1\n' "${install_dir}" "${arch}" "${install_dir}" "${arch}" "${opts}" > "${isofs_dir}/loader/entries/02-bifrost-live.conf"
    efiboot_files=("${isofs_dir}/EFI/" "${isofs_dir}/loader/" "${boot_dir}/vmlinuz-"* "${boot_dir}/initramfs-"*".img")
    _make_efibootimg
    mcopy -s -i "${efibootimg}" "${isofs_dir}/EFI" "${isofs_dir}/loader" '::/'
    # the loader starts the kernel's EFI stub from the ESP it was launched from; it passes one initrd, so separate
    # microcode images (added when the initramfs has no microcode hook) are not copied
    mmd -i "${efibootimg}" "::/${install_dir}" "::/${install_dir}/boot" "::/${install_dir}/boot/${arch}"
    mcopy -i "${efibootimg}" "${boot_dir}/vmlinuz-"* "${boot_dir}/initramfs-"*".img" "::/${install_dir}/boot/${arch}/"
    _msg_info "Done! BifrOSt EFI loader set up."
}

_validate_requirements_bootmode_uefi.bifrost() {
    local repo="${BIFROST_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
    if [[ ! -s "${repo}/packaging/archiso/bootweave.efi" && ! -s "${repo}/packaging/arch/recovery/bootweave.efi" ]]; then
        (( validation_error=validation_error+1 ))
        _msg_error "packaging/archiso/bootweave.efi and packaging/arch/recovery/bootweave.efi are missing (run tools/build-efi-boot.sh)" 0
    fi
    _validate_common_requirements_bootmode_uefi
}

_add_xorrisofs_options_uefi.bifrost() {
    _add_common_xorrisofs_options_uefi
}

'''


def main() -> int:
    if not SOURCE.is_file():
        print(f"Archiso build tool not found: {SOURCE}", file=sys.stderr)
        return 2
    source = SOURCE.read_text()
    marker = "while getopts 'c:p:C:L:P:A:D:w:m:o:g:G:vrh?' arg; do"
    if source.count(marker) != 1:
        print("mkarchiso changed; refusing to patch an unknown version", file=sys.stderr)
        return 2
    patched = source.replace(marker, INJECT + marker)
    DEST.parent.mkdir(parents=True, exist_ok=True)
    DEST.write_text(patched)
    DEST.chmod(0o755)
    if len(sys.argv) == 1:
        print(DEST)
        return 0
    os.execv(str(DEST), [str(DEST), *sys.argv[1:]])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
