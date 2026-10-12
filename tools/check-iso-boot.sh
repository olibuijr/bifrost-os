#!/usr/bin/env bash
# Fail unless the ISO boots through the BifrOSt EFI loader alone: UEFI El Torito only, no syslinux,
# systemd-boot, GRUB, memtest, EFI shell or Plymouth. usage: check-iso-boot.sh ISO
set -euo pipefail
ISO=${1:?usage: $0 ISO}
cd "$(dirname "$0")/.."
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
bad=0
xorriso -indev "$ISO" -report_el_torito plain 2>/dev/null | grep -q 'El Torito boot img : *[0-9]* *BIOS' && { echo "FOUND BIOS El Torito entry"; bad=1; }
tree=$(xorriso -indev "$ISO" -find / 2>/dev/null)
for p in /boot/syslinux /boot/memtest86+ /shellx64.efi /EFI/BOOT/BOOTIA32.EFI; do
  grep -qx "'$p'" <<<"$tree" && { echo "FOUND $p"; bad=1; }
done
# mkarchiso always writes /boot/grub/grubenv (data for loopback booting); any other GRUB file is a loader
grep "^'/boot/grub/" <<<"$tree" | grep -qv "^'/boot/grub/grubenv'$" && { echo "FOUND GRUB files"; bad=1; }
for p in /EFI/BOOT/BOOTx64.efi /EFI/BIFROST/BOOTx64.efi /EFI/BIFROST/boot.cfg; do
  grep -qx "'$p'" <<<"$tree" || { echo "MISSING $p"; bad=1; }
done
# the firmware boots the El Torito ESP image, not the ISO 9660 tree: its loader must be ours byte for byte
xorriso -osirrox on -indev "$ISO" -extract_boot_images "$T/" >/dev/null 2>&1 || true
esp=$(ls "$T"/eltorito_img*_uefi.img 2>/dev/null | head -1)
if [ -z "$esp" ]; then echo "MISSING UEFI boot image"; bad=1
else
  for p in EFI/BOOT/BOOTx64.efi EFI/BIFROST/BOOTx64.efi; do
    mcopy -n -i "$esp" "::/$p" "$T/l.efi" 2>/dev/null && cmp -s "$T/l.efi" packaging/archiso/bootweave.efi || { echo "ESP $p is not the BifrOSt loader"; bad=1; }
    rm -f "$T/l.efi"
  done
  mdir -/ -b -i "$esp" :: 2>/dev/null | grep -iE 'systemd|loader\.conf|shell.*\.efi|memtest' && { echo "FOUND foreign boot files in ESP"; bad=1; }
fi
bsdtar -xOf "$ISO" arch/x86_64/airootfs.sfs > "$T/root.sfs"
n=$(unsquashfs -l "$T/root.sfs" 2>/dev/null | grep -ci plymouth || true)
[ "${n:-0}" = 0 ] || { echo "FOUND $n Plymouth paths"; bad=1; }
[ "$bad" = 0 ] && echo "OK: $ISO boots through the BifrOSt loader only" || { echo "FAILED: $ISO"; exit 1; }
