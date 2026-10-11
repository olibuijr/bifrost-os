#!/usr/bin/env bash
# Headless BifrOSt install into the x86 QEMU VM (no clicking, no ISO rebuild): boots the newest lab ISO, copies the
# freshly built installer in, runs it with BIFROST_AUTO and boots the installed disk.
# usage: tools/vm-install.sh [install|boot|ssh CMD...]   env: ISO=path (default: newest out/iso-lab-*/ *.iso), USER_PW=test:test
#   install  build the installer (fast profile), boot the ISO in the VM, install onto the VM's blank SCSI disk (by WWN)
#   boot     boot the installed disk (no ISO) on Hyprland workspace 2
#   ssh CMD  run CMD as root in the live VM over the lab key (port 2252)
set -euo pipefail
cd "$(dirname "$0")/.."
SSH=(ssh -i out/lab/id_ed25519 -p 2252 -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o BatchMode=yes -o ConnectTimeout=3 -o LogLevel=ERROR root@127.0.0.1)
case "${1:-install}" in
  install)
    (cd springboard && cargo build --profile fast --bin installer)
    pkill -f 'name BifrOSt Installer' || true; sleep 2
    ISO=${ISO:-$(ls -t out/iso-lab-*/*.iso | head -1)} tools/arch-vm-iso.sh >/dev/null 2>&1
    for _ in $(seq 60); do "${SSH[@]}" true 2>/dev/null && break; sleep 2; done
    scp -i out/lab/id_ed25519 -P 2252 -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o LogLevel=ERROR springboard/target/fast/installer root@127.0.0.1:/tmp/inst-auto
    "${SSH[@]}" "systemctl stop bifrost-installer; chmod +x /tmp/inst-auto; (BIFROST_TRACE=1 BIFROST_AUTO=${USER_PW:-test:test}:/dev/sda setsid /tmp/inst-auto > /tmp/auto.log 2>&1 &)"
    echo "installing; follow with: tools/vm-install.sh ssh \"grep -a 'bifrost-installer: \\[' /tmp/auto.log | tail -3\"" ;;
  boot)
    pkill -f 'name BifrOSt Installer' || true; sleep 2
    ISO_NONE=1 KEEP_VARS=1 tools/arch-vm-iso.sh >/dev/null 2>&1 ;;
  ssh) shift; "${SSH[@]}" "$@" ;;
  *) echo "usage: $0 [install|boot|ssh CMD]" >&2; exit 2 ;;
esac
