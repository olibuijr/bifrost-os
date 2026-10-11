#!/usr/bin/env bash
# Boot the BifrOSt installer ISO in a headed UEFI VM on Hyprland workspace 2, fullscreen,
# with a blank 16G target disk. ISO_BOOT_IMG (optional) is a FAT image tried before the ISO.
set -euo pipefail
cd "$(dirname "$0")/.."
V=out/arch-vm; ISO=${ISO:-$(ls -t out/iso/*.iso | head -1)}
[ -f $V/installer-target.raw ] || truncate -s 16G $V/installer-target.raw
[ -n "${KEEP_VARS:-}" ] || cp /usr/share/edk2/x64/OVMF_VARS.4m.fd $V/installer-vars.fd
cd=(-drive file="$ISO",media=cdrom,if=none,id=cd,readonly=on -device ide-cd,drive=cd,bootindex=2); [ -z "${ISO_NONE:-}" ] || cd=()
extra=(); [ -z "${ISO_BOOT_IMG:-}" ] || extra=(-drive file="$ISO_BOOT_IMG",if=none,id=bt,format=raw -device usb-ehci -device usb-storage,drive=bt,bootindex=1)
export HYPRLAND_INSTANCE_SIGNATURE=$(ls -t /run/user/1000/hypr | head -1) XDG_RUNTIME_DIR=/run/user/1000
export WAYLAND_DISPLAY=$(ls /run/user/1000 | grep '^wayland-[0-9]*$' | head -1) GDK_BACKEND=wayland
# HEADLESS=1: no window (screens come from QMP screendump and ssh), so no Wayland session is needed
DISP=(-display gtk,zoom-to-fit=on)
GPU=(-device virtio-vga,xres=1280,yres=720)
if [ "${VM_GL:-0}" = 1 ]; then
  GPU=(-device virtio-vga-gl,xres=1280,yres=720)
  DISP=(-display gtk,gl=on,zoom-to-fit=on)
  [ -z "${HEADLESS:-}" ] || DISP=(-display egl-headless)
else
  [ -z "${HEADLESS:-}" ] || DISP=(-display none)
fi
# virtio-tablet-pci is the QEMU absolute-pointer device id
nohup setsid qemu-system-x86_64 -name "BifrOSt Installer" -machine q35,accel=kvm -cpu host -smp 4 -m "${VM_RAM:-4096}" \
 -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd -drive if=pflash,format=raw,unit=1,file=$V/installer-vars.fd \
 ${ISO_NONE:+} "${cd[@]}" "${extra[@]}" \
 -drive file=$V/installer-target.raw,if=virtio,format=raw "${GPU[@]}" -device virtio-keyboard-pci -device virtio-tablet-pci \
 -virtfs local,path=packaging/archiso/airootfs,mount_tag=dev,security_model=none,readonly=on -netdev user,id=n0,hostfwd=tcp:127.0.0.1:2252-:22 -device virtio-net-pci,netdev=n0 "${DISP[@]}" -audio none \
 -chardev socket,id=s0,path=$V/installer-serial.sock,server=on,wait=off,logfile=$V/installer-serial.log -serial chardev:s0 -qmp unix:$V/installer-qmp.sock,server=on,wait=off >$V/installer-qemu.log 2>&1 </dev/null &
[ -z "${HEADLESS:-}" ] || exit 0
for _ in $(seq 50); do
  A=$(hyprctl clients -j | python3 -c "import json,sys;print(next((c['address'] for c in json.load(sys.stdin) if c['title']=='QEMU (BifrOSt Installer)'),''))")
  [ -n "$A" ] && break; sleep .2
done
hyprctl dispatch "hl.dsp.window.move({ workspace = \"2\", window = \"address:$A\" })" >/dev/null
hyprctl dispatch 'hl.dsp.focus({ workspace = "2" })' >/dev/null
hyprctl dispatch "hl.dsp.window.fullscreen({ action = \"set\", window = \"address:$A\" })" >/dev/null
echo started
