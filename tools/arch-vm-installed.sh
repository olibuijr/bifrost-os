#!/usr/bin/env bash
# Boot the installed raw disk image in QEMU on Hyprland workspace 2.
# The guest sees only its private disk image and user-mode networking.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
VMDIR=$REPO/out/arch-vm
DISK=$VMDIR/bifrost-install-target.raw
VARS=$VMDIR/install-vars.fd
QMP=$VMDIR/install-boot-qmp.sock
PIDF=$VMDIR/install-boot.pid
SERIAL=$VMDIR/install-boot-serial.log
LOG=$VMDIR/install-boot-qemu.log
KEY=$VMDIR/id_ed25519
SSH_PORT=2252
TITLE="BifrOSt Installed Disk"
CODE=/usr/share/edk2/x64/OVMF_CODE.4m.fd
VARS_TEMPLATE=/usr/share/edk2/x64/OVMF_VARS.4m.fd

running() {
  [ -f "$PIDF" ] || return 1
  local pid
  pid=$(cat "$PIDF")
  kill -0 "$pid" 2>/dev/null || return 1
  tr '\0' ' ' <"/proc/$pid/cmdline" 2>/dev/null | grep -Fq -- "$DISK"
}

hypr() {
  command -v hyprctl >/dev/null 2>&1 || { echo "hyprctl is unavailable" >&2; return 1; }
  if [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
    local -a instances
    mapfile -t instances < <(find "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/hypr" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null)
    [ "${#instances[@]}" -eq 1 ] || { echo "Set HYPRLAND_INSTANCE_SIGNATURE for the active Hyprland session." >&2; return 1; }
    export HYPRLAND_INSTANCE_SIGNATURE=${instances[0]}
  fi
  export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
  if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    [ -S "$XDG_RUNTIME_DIR/wayland-1" ] || { echo "WAYLAND_DISPLAY is unset and wayland-1 is unavailable." >&2; return 1; }
    export WAYLAND_DISPLAY=wayland-1
  fi
}

hypr_dispatch() {
  hyprctl dispatch "$1" >/dev/null
}

start() {
  if running; then echo "already running (pid $(cat "$PIDF"))"; return 0; fi
  [ -f "$DISK" ] && [ ! -L "$DISK" ] || { echo "missing regular install image: $DISK" >&2; return 1; }
  [ -f "$KEY" ] && [ ! -L "$KEY" ] || { echo "missing VM key: $KEY" >&2; return 1; }
  [ -f "$CODE" ] && [ -f "$VARS_TEMPLATE" ] || { echo "OVMF firmware is not installed" >&2; return 1; }
  if [ ! -f "$VARS" ]; then install -m600 "$VARS_TEMPLATE" "$VARS"; fi
  [ ! -L "$VARS" ] || { echo "refusing symlinked UEFI variables image" >&2; return 1; }
  chmod 600 "$VARS"
  rm -f "$QMP"
  : >"$SERIAL"
  hypr
  # Hyprland 0.55+ uses its Lua dispatcher API (the old string form fails on
  # this host's 0.56.2 Lua configuration).
  hypr_dispatch 'hl.dsp.focus({ workspace = "2" })'
  # virtio-tablet-pci is the QEMU absolute-pointer device id
  GDK_BACKEND=wayland nohup setsid qemu-system-x86_64 -name "$TITLE" \
    -machine q35,accel=kvm -cpu host -smp "${BIFROST_INSTALL_CPUS:-4}" -m "${BIFROST_INSTALL_MEMORY:-3072}" \
    -drive if=pflash,format=raw,unit=0,readonly=on,file="$CODE" \
    -drive if=pflash,format=raw,unit=1,file="$VARS" \
    -drive "file=$DISK,if=virtio,format=raw" \
    -device virtio-vga,xres=1280,yres=720 -device virtio-keyboard-pci -device virtio-tablet-pci \
    -device virtio-balloon-pci,free-page-reporting=on,deflate-on-oom=on \
    -netdev user,id=installednet,hostfwd=tcp:127.0.0.1:$SSH_PORT-:22 -device virtio-net-pci,netdev=installednet \
    -display gtk,zoom-to-fit=on -audio none -serial "file:$SERIAL" \
    -qmp "unix:$QMP,server=on,wait=off" -pidfile "$PIDF" >"$LOG" 2>&1 </dev/null &

  local address=""
  for _ in $(seq 1 100); do
    running || { echo "QEMU exited; see $LOG" >&2; tail -20 "$LOG"; return 1; }
    address=$(hyprctl -j clients | python3 -c 'import json,sys; title=sys.argv[1]; clients=json.load(sys.stdin); print(next((c["address"] for c in clients if c.get("title")==title), ""))' "$TITLE")
    [ -n "$address" ] && break
    sleep 0.2
  done
  [ -n "$address" ] || { echo "QEMU window did not appear; see $LOG" >&2; return 1; }
  hypr_dispatch "hl.dsp.window.move({ workspace = \"2\", window = \"address:$address\" })"
  hypr_dispatch 'hl.dsp.focus({ workspace = "2" })'
  hypr_dispatch "hl.dsp.focus({ window = \"address:$address\" })"
  hypr_dispatch "hl.dsp.window.fullscreen({ action = \"set\", window = \"address:$address\" })"
  echo "started installed disk on Hyprland workspace 2, fullscreen (pid $(cat "$PIDF"))"
}

case "${1:-}" in
  start) start ;;
  status)
    if running; then echo "running pid $(cat "$PIDF"), disk=$DISK, ssh=127.0.0.1:$SSH_PORT"; else echo "stopped"; fi ;;
  shot)
    running || { echo "not running" >&2; exit 1; }
    python3 "$HERE/arch-vm-qmp.py" "$QMP" 1280 720 shot "${2:-$VMDIR/installed-disk.png}"
    ;;
  stop)
    running || { echo "stopped"; exit 0; }
    ssh -i "$KEY" -p "$SSH_PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR -o ConnectTimeout=3 -o BatchMode=yes root@127.0.0.1 systemctl poweroff >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do running || { echo "stopped"; exit 0; }; sleep 1; done
    echo "guest did not power off; leaving QEMU running" >&2; exit 1
    ;;
  *) echo "Usage: $0 {start|status|shot [OUT.png]|stop}"; exit 2 ;;
esac
