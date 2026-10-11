#!/usr/bin/env bash
# Keep only the newest N images (default 1: one ISO in the iVentoy menu) in iVentoy's iso/ folder. Refuses to delete anything while a client is
# connected to iVentoy's image services (iSCSI 3260, NBD 10809, NFS 12049, SMB 10445, HTTP 16000): replacing a file a
# machine is booted from breaks its installation. usage: tools/iventoy-prune.sh [N]
set -euo pipefail
cd "$(dirname "$0")/.."
KEEP=${1:-1}; D=out/iventoy/iventoy-1.0.44/iso
if ss -tn state established '( sport = :3260 or sport = :10809 or sport = :12049 or sport = :10445 or sport = :16000 )' 2>/dev/null | tail -n +2 | grep -q .; then
  echo "iventoy-prune: a client is connected to iVentoy, not deleting anything"; exit 0
fi
mapfile -t OLD < <(ls -t "$D"/*.iso 2>/dev/null | tail -n +$((KEEP + 1)))
for f in "${OLD[@]}"; do
  echo "iventoy-prune: removing $(basename "$f")"; rm -f -- "$f"; done
curl -s http://127.0.0.1:26000/iventoy/json -d '{"method":"refresh_img_list"}' >/dev/null || true
echo "iventoy-prune: kept $(ls "$D"/*.iso | xargs -n1 basename | tr '\n' ' ')"
