#!/usr/bin/env bash
# TFTP for Midget's VLAN 20 ("Midget Lab") netboot: serves iPXE over the home NIC (192.168.1.12).
# The UniFi network 6ac940ef2f3dfdb189cf3726 hands out next-server 192.168.1.12, filename ipxe.efi.
#   tools/midget-tftp.sh up      start (transient unit midget-tftp)
#   tools/midget-tftp.sh down    stop
set -euo pipefail
case "${1:-up}" in
  up)
    sudo mkdir -p /srv/tftp
    sudo ln -sfn /usr/share/ipxe/x86_64/ipxe.efi /srv/tftp/ipxe.efi
    sudo systemctl stop midget-tftp 2>/dev/null || true
    sudo systemd-run --unit=midget-tftp --description="Midget lab TFTP (ipxe.efi)" --property=Restart=on-failure \
      /usr/bin/dnsmasq --keep-in-foreground --port=0 --enable-tftp --tftp-root=/srv/tftp \
      --interface=enp6s0 --bind-interfaces --pid-file=/run/midget-tftp.pid
    ;;
  down) sudo systemctl stop midget-tftp ;;
  *) echo "usage: $0 up|down"; exit 2 ;;
esac
