#!/usr/bin/env bash
# Bring the midget netboot lab up (idempotent; run after a titan reboot): the lab NIC address, NAT for the
# lab network, and iVentoy bound to the lab NIC only. Never touches the home LAN interface.
#   tools/lab-up.sh          start        tools/lab-up.sh down      stop iVentoy and remove the NAT rules
set -euo pipefail
cd "$(dirname "$0")/.."
LAB_NIC=${LAB_NIC:-eno2}; LAB_IP=10.99.0.1; LAN_NIC=${LAN_NIC:-enp6s0}
IV=out/iventoy/iventoy-1.0.44
if [ "${1:-up}" = down ]; then
  sudo "$IV/iventoy.sh" stop || true
  sudo nft delete table ip bifrost_nat 2>/dev/null || true
  sudo iptables -D DOCKER-USER -i "$LAB_NIC" -o "$LAN_NIC" -j ACCEPT 2>/dev/null || true
  sudo iptables -D DOCKER-USER -i "$LAN_NIC" -o "$LAB_NIC" -m state --state ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true
  echo "lab down"; exit 0
fi
ip -4 addr show "$LAB_NIC" | grep -q "$LAB_IP/" || sudo ip addr add "$LAB_IP/24" dev "$LAB_NIC"
# Titan e1000e TX queue hangs under large OTA transfers (2026-10-08).
# Restrict this reversible workaround to the lab NIC.
if command -v ethtool >/dev/null; then
  sudo ethtool -K "$LAB_NIC" tso off gso off gro off || true
  sudo ethtool --set-eee "$LAB_NIC" eee off || true
fi
sudo sysctl -q -w net.ipv4.ip_forward=1
if ! sudo nft list table ip bifrost_nat >/dev/null 2>&1; then
  sudo nft add table ip bifrost_nat
  sudo nft "add chain ip bifrost_nat post { type nat hook postrouting priority 100; }"
  sudo nft add rule ip bifrost_nat post ip saddr 10.99.0.0/24 oifname "$LAN_NIC" masquerade
fi
# Docker's FORWARD policy would drop the lab traffic
sudo iptables -C DOCKER-USER -i "$LAB_NIC" -o "$LAN_NIC" -j ACCEPT 2>/dev/null || sudo iptables -I DOCKER-USER -i "$LAB_NIC" -o "$LAN_NIC" -j ACCEPT
sudo iptables -C DOCKER-USER -i "$LAN_NIC" -o "$LAB_NIC" -m state --state ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || sudo iptables -I DOCKER-USER -i "$LAN_NIC" -o "$LAB_NIC" -m state --state ESTABLISHED,RELATED -j ACCEPT
sudo "$IV/iventoy.sh" status 2>/dev/null | grep -q running || sudo "$IV/iventoy.sh" start
for _ in $(seq 20); do curl -s http://127.0.0.1:26000/ >/dev/null && break; sleep 0.5; done
# start the PXE service on the lab NIC ONLY (the UI defaults to the LAN interface; never use that)
curl -s http://127.0.0.1:26000/iventoy/json -H 'Content-Type: application/json' \
  -d "{\"method\":\"start_server\",\"ip\":\"$LAB_IP\",\"mask\":\"255.255.255.0\",\"gw\":\"$LAB_IP\",\"pool_begin\":\"10.99.0.200\",\"pool_end\":\"10.99.0.219\",\"dns_server\":\"1.1.1.1\",\"nic\":\"$LAB_NIC\"}"; echo
# unattended netboot (MCP lifecycle netboot/reinstall): the newest lab ISO boots after 5 s without a keypress.
# iVentoy only accepts config changes while its PXE service is stopped.
J() { curl -s -m10 http://127.0.0.1:26000/iventoy/json -H 'Content-Type: application/json' -d "$1"; }
"$(dirname "$0")"/iventoy-prune.sh 1 || true   # only the latest image exists (owner rule)
NEWEST=$(ls -t "$IV"/iso/bifrost-lab*.iso 2>/dev/null | head -1 | xargs -r basename)
if [[ -n $NEWEST ]]; then
  J '{"method":"stop_server"}' >/dev/null; J '{"method":"refresh_img_list"}' >/dev/null; sleep 2
  ID=$(J '{"method":"get_img_tree"}' | python3 -c "import sys,json; print(next((i['imgid'] for i in json.load(sys.stdin) if i['name']=='$NEWEST'),''))")
  [[ -n $ID ]] && J "{\"method\":\"set_default_image\",\"id\":$ID}" >/dev/null
  J '{"method":"set_default_timeout","deftimeout":5}' >/dev/null
  J "{\"method\":\"start_server\",\"ip\":\"$LAB_IP\",\"mask\":\"255.255.255.0\",\"gw\":\"$LAB_IP\",\"pool_begin\":\"10.99.0.200\",\"pool_end\":\"10.99.0.219\",\"dns_server\":\"1.1.1.1\",\"nic\":\"$LAB_NIC\"}"; echo
fi
echo "lab up: iVentoy on $LAB_NIC ($LAB_IP), default image ${NEWEST:-none} after 5 s"
