#!/usr/bin/env bash
# Arch Linux x86_64 guest under QEMU/KVM with virtio-gpu (real DRM/KMS), running the SpringBoard shell as the sole UI.
# Usage: arch-vm.sh {install-deps|create|start [--headed]|stop|status|shot OUT.png|key KEY|tap X Y|
#                    swipe X0 Y0 X1 Y1 [MS]|ssh CMD...|deploy}
# Docs: docs/arch-vm.md. User-mode networking only, no audio, nothing written to /tmp.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
VMDIR=${ARCH_VM_DIR:-$HOME/.local/share/arch-vm}
export TMPDIR=$VMDIR/tmp
mkdir -p "$TMPDIR"
DISK=$VMDIR/arch.img
DISK_SIZE=${ARCH_VM_DISK:-6G}
KERNEL=$VMDIR/vmlinuz
INITRD=$VMDIR/initramfs.img
KEY=$VMDIR/id_ed25519
QMP=$VMDIR/qmp.sock
PIDF=$VMDIR/qemu.pid
LOG=$VMDIR/qemu.log
SERIAL=$VMDIR/serial.log
SSH_PORT=${ARCH_VM_SSH_PORT:-2250}
W=${ARCH_VM_W:-1200}
H=${ARCH_VM_H:-1920}
# The shell and base OS fit comfortably in 1 GiB; override for heavier app workloads.
MEM=${ARCH_VM_MEM:-1024}
CPUS=${ARCH_VM_CPUS:-4}
SHOTS=$HOME/Pictures/tab
BIN=${SPRINGBOARD_BIN:-$REPO/springboard/target/release/shell}
ASSETS=${SPRINGBOARD_ASSETS:-$REPO/springboard/assets}

running() { [ -f "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null; }
vssh() {
  ssh -i "$KEY" -p "$SSH_PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes root@127.0.0.1 "$@"
}
qmp() { python3 -I "$HERE/arch-vm-qmp.py" "$QMP" "$W" "$H" "$@"; }

case "${1:-}" in
install-deps)
  need=()
  for p in qemu-desktop edk2-ovmf arch-install-scripts; do pacman -Q "$p" >/dev/null 2>&1 || need+=("$p"); done
  # qemu-base alone (no qemu-desktop) also provides the binary but lacks the gtk/sdl displays
  if [ ${#need[@]} -gt 0 ]; then sudo pacman -S --needed --noconfirm "${need[@]}" || exit 1; else echo "deps present"; fi ;;

create)
  mkdir -p "$VMDIR"
  command -v pacstrap >/dev/null || { echo "run install-deps first"; exit 1; }
  [ -f "$DISK" ] && { echo "$DISK exists; remove it to recreate"; exit 0; }
  [ -f "$KEY" ] || ssh-keygen -q -t ed25519 -N '' -C arch-vm -f "$KEY"
  mnt=$VMDIR/mnt
  mkdir -p "$mnt"
  truncate -s "$DISK_SIZE" "$DISK.part" && mkfs.ext4 -q -F "$DISK.part" || exit 1
  sudo mount -o loop "$DISK.part" "$mnt" || exit 1
  ok=0
  if sudo pacstrap -K "$mnt" base linux openssh; then
    sudo install -d -m700 "$mnt/root/.ssh"
    sudo install -m600 "$KEY.pub" "$mnt/root/.ssh/authorized_keys"
    sudo tee "$mnt/etc/systemd/network/20-wired.network" >/dev/null <<'NET'
[Match]
Name=en* eth*
[Network]
DHCP=yes
NET
    echo springboard-vm | sudo tee "$mnt/etc/hostname" >/dev/null
    printf 'PermitRootLogin prohibit-password\n' | sudo tee "$mnt/etc/ssh/sshd_config.d/10-root-key.conf" >/dev/null
    sudo arch-chroot "$mnt" systemctl enable sshd.service systemd-networkd.service systemd-resolved.service >/dev/null
    sudo arch-chroot "$mnt" systemctl set-default graphical.target >/dev/null
    sudo ln -sf /run/systemd/resolve/stub-resolv.conf "$mnt/etc/resolv.conf"
    # the pacman mkinitcpio hook fails inside pacstrap's chroot, so build the image here; skip autodetect so
    # every module (virtio_gpu, virtio_blk, ...) is included
    kver=$(sudo ls "$mnt/usr/lib/modules" | head -1)
    sudo arch-chroot "$mnt" mkinitcpio -k "$kver" -g /boot/initramfs.img -S autodetect >/dev/null &&
      sudo install -m644 -o "$(id -u)" "$mnt/usr/lib/modules/$kver/vmlinuz" "$KERNEL" &&
      sudo install -m644 -o "$(id -u)" "$mnt/boot/initramfs.img" "$INITRD" && ok=1
  fi
  sudo fuser -k -m "$mnt" >/dev/null 2>&1   # pacstrap -K leaves gpg-agent/dirmngr holding the mount
  sudo umount "$mnt" || sudo umount -l "$mnt"
  if [ $ok = 1 ]; then mv "$DISK.part" "$DISK"; echo "created $DISK"; else rm -f "$DISK.part"; echo "create failed"; exit 1; fi ;;

start)
  running && { echo "already running (pid $(cat "$PIDF"))"; exit 0; }
  [ -f "$DISK" ] || { echo "no disk; run create first"; exit 1; }
  display=(-display none)
  [ "${2:-}" = --headed ] && display=(-display gtk,zoom-to-fit=on -name "Arch VM :$SSH_PORT")
  rm -f "$QMP"
  : >"$SERIAL"
  # virtio-tablet-pci is the QEMU absolute-pointer device id
  nohup setsid qemu-system-x86_64 -name springboard-vm -machine q35,accel=kvm -cpu host -smp "$CPUS" -m "$MEM" \
    -kernel "$KERNEL" -initrd "$INITRD" \
    -append "root=/dev/vda rw console=ttyS0 console=tty1 loglevel=4 systemd.show_status=0" \
    -drive file="$DISK",if=virtio,format=raw \
    -device virtio-vga,xres="$W",yres="$H" -device virtio-keyboard-pci -device virtio-tablet-pci \
    -device virtio-balloon-pci,free-page-reporting=on,deflate-on-oom=on \
    -netdev user,id=n0,hostfwd=tcp:127.0.0.1:"$SSH_PORT"-:22 -device virtio-net-pci,netdev=n0 \
    "${display[@]}" -audio none -serial file:"$SERIAL" -qmp unix:"$QMP",server=on,wait=off \
    -pidfile "$PIDF" >"$LOG" 2>&1 </dev/null &
  t0=$SECONDS
  for _ in $(seq 1 60); do
    running || { sleep 1; running || { echo "qemu died; see $LOG"; tail -5 "$LOG"; exit 1; }; }
    vssh true 2>/dev/null && { echo "ssh up after $((SECONDS-t0))s (127.0.0.1:$SSH_PORT)"; exit 0; }
    sleep 2
  done
  echo "timeout waiting for ssh; see $SERIAL"; exit 1 ;;

stop)
  if running; then
    qmp key power >/dev/null 2>&1 || true
    vssh systemctl poweroff >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do running || break; sleep 1; done
    running && kill "$(cat "$PIDF")"
  fi
  rm -f "$PIDF" "$QMP"; echo stopped ;;

status)
  if running; then
    echo "running pid $(cat "$PIDF") ssh=127.0.0.1:$SSH_PORT"
    vssh 'systemctl is-active springboard.service; systemctl show -p NRestarts springboard.service' 2>&1
  else echo "stopped"; fi ;;

shot)
  out=${2:?usage: shot OUT.png}
  running || { echo "not running"; exit 1; }
  mkdir -p "$SHOTS"
  case "$out" in /*) ;; *) out=$SHOTS/$out ;; esac
  qmp shot "$out" && echo "$out" ;;

key) qmp key "${2:?KEY}" ;;
tap) qmp tap "${2:?X}" "${3:?Y}" ;;
swipe) shift; qmp swipe "$@" ;;
ssh) shift; vssh "$@" ;;

deploy)
  running || { echo "not running"; exit 1; }
  [ -x "$BIN" ] || { echo "no shell binary at $BIN (cargo build --release --bin shell in springboard/)"; exit 1; }
  vssh 'mkdir -p /usr/lib/springboard /usr/share/springboard' || exit 1
  vssh 'cat > /usr/bin/springboard.new && chmod 755 /usr/bin/springboard.new' <"$BIN" || exit 1
  vssh 'cat > /usr/lib/systemd/system/springboard.service' <"$REPO/packaging/arch/springboard.service" || exit 1
  vssh 'cat > /usr/lib/springboard/seed-data && chmod 755 /usr/lib/springboard/seed-data' <"$REPO/packaging/arch/seed-data" || exit 1
  if [ -d "$ASSETS" ] && ! vssh 'test -e /usr/share/springboard/assets/.deployed'; then
    tar -C "$ASSETS" -cf - . | vssh 'mkdir -p /usr/share/springboard/assets && tar -C /usr/share/springboard/assets -xf - && touch /usr/share/springboard/assets/.deployed' || exit 1
  fi
  vssh 'mv /usr/bin/springboard.new /usr/bin/springboard && systemctl daemon-reload && systemctl enable springboard.service >/dev/null 2>&1; systemctl restart springboard.service; sleep 3; systemctl is-active springboard.service' ;;

*) sed -n '2,4p' "$0"; exit 2 ;;
esac
